//! `architecture.describe` derives the graph from the tree; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §2.

use std::fs;
use std::path::PathBuf;

use agent::{Instance, Request, Response, run_with};

use crate::harness as common;
use serde_json::Value;
use tempfile::TempDir;

const MANIFEST: &str = "description: A test instance.\nschema: v0\nname: test\nrepository: https://example.invalid/test\npaths:\n  policies: .config/policies\n  providers: .config/providers.yaml\n  releases: .config/releases.yaml\n";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn describe(root: &std::path::Path) -> Response {
    let instance = Instance::load(root).expect("valid manifest");
    run_with(
        &instance,
        &Request {
            operation: "architecture.describe".into(),
            ..Request::default()
        },
        common::var,
    )
    .expect("runs")
}

fn ids(r: &Response, key: &str) -> Vec<String> {
    r.result[key]
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn this_repository_is_described_the_same_way_twice() {
    let first = describe(&repo_root());
    assert!(first.ok, "{:?}", first.violations);
    let again = describe(&repo_root());
    assert_eq!(
        serde_json::to_string(&first).expect("json"),
        serde_json::to_string(&again).expect("json")
    );
    let nodes = ids(&first, "nodes");
    for want in [
        "engine:cobot",
        "operation:architecture.describe",
        "policy:change",
        "repository",
        "unit:infra/pigeon",
        "provider:cloudflare",
    ] {
        assert!(
            nodes.iter().any(|n| n == want),
            "{want} is missing: {nodes:?}"
        );
    }
    assert!(
        first.result["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .all(|n| n["source"].as_str().is_some_and(|s| !s.is_empty())),
        "every node names its source"
    );
    let placed = ids(&first, "nodes").len();
    assert_eq!(
        first.result["layout"]["nodes"].as_array().map(Vec::len),
        Some(placed)
    );
}

#[test]
fn no_bucket_name_reaches_the_output() {
    let text = serde_json::to_string(&describe(&repo_root())).expect("json");
    let config: Value = serde_saphyr::from_str(
        &fs::read_to_string(repo_root().join("infra/pigeon/config.yaml")).expect("read"),
    )
    .expect("yaml");
    let names: Vec<&str> = config["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|b| b["name"].as_str())
        .collect();
    assert_ne!(names, Vec::<&str>::new());
    for name in names {
        assert!(!text.contains(name), "{name} is in the output");
    }
}

#[test]
fn a_new_policy_adds_one_node_and_one_edge() {
    let root = TempDir::new().expect("tempdir");
    let config = root.path().join(".config");
    fs::create_dir_all(config.join("policies")).expect("mkdir");
    fs::write(config.join("instance.yaml"), MANIFEST).expect("write");
    fs::write(config.join("providers.yaml"), "").expect("write");
    fs::write(config.join("releases.yaml"), "").expect("write");
    let policy = "description: p.\ntarget: tree\nrules:\n  - entries: { path: ., dirs: ['.'], files: ['.'] }\n";
    fs::write(config.join("policies/a.yaml"), policy).expect("write");
    let before = describe(root.path());

    fs::write(config.join("policies/b.yaml"), policy).expect("write");
    let after = describe(root.path());
    let count = |r: &Response, key: &str| r.result[key].as_array().map_or(0, Vec::len);
    assert_eq!(count(&after, "nodes"), count(&before, "nodes") + 1);
    assert_eq!(count(&after, "edges"), count(&before, "edges") + 1);
    assert!(ids(&after, "nodes").contains(&"policy:b".to_string()));

    fs::remove_file(config.join("policies/b.yaml")).expect("rm");
    assert_eq!(
        serde_json::to_string(&describe(root.path())).expect("json"),
        serde_json::to_string(&before).expect("json"),
        "removing it restores the output byte for byte"
    );
}

/// Copies this repository's configuration and units, without sites' content, into a tempdir.
fn units_fixture() -> TempDir {
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        fs::create_dir_all(to).expect("mkdir");
        for e in fs::read_dir(from).expect("list").flatten() {
            let name = e.file_name();
            if e.path().is_dir() {
                if name != "site" {
                    copy(&e.path(), &to.join(&name));
                }
            } else {
                fs::copy(e.path(), to.join(&name)).expect("copy");
            }
        }
    }
    let root = TempDir::new().expect("tempdir");
    for dir in [".config", "catalog", "infra", "workloads"] {
        copy(&repo_root().join(dir), &root.path().join(dir));
    }
    root
}

#[test]
fn an_invalid_unit_is_a_violation() {
    let root = units_fixture();
    assert!(describe(root.path()).ok, "the copy describes cleanly");
    let blog = root
        .path()
        .join("workloads/noisypigeon.com/blog/config.yaml");
    let text = fs::read_to_string(&blog).expect("read");
    let cut = text.find("  site:").expect("a site block");
    fs::write(&blog, &text[..cut]).expect("write");
    let r = describe(root.path());
    assert!(!r.ok);
    assert!(
        r.violations
            .iter()
            .any(|v| v.contains("workloads/noisypigeon.com/blog")),
        "{:?}",
        r.violations
    );
}

#[test]
fn a_credential_draws_its_capabilities_and_a_unit_dir_policy_its_unit() {
    let root = units_fixture();
    let schema = root.path().join("infra/pigeon/schema.json");
    let mut s: Value =
        serde_json::from_str(&fs::read_to_string(&schema).expect("read")).expect("json");
    s["x-pigeon"]["credentials"] = serde_json::json!({
        "deploy": { "provider": "cloudflare", "scope": "account", "capabilities": ["r2.write"] }
    });
    fs::write(&schema, serde_json::to_string_pretty(&s).expect("json")).expect("write");
    fs::write(
        root.path().join(".config/policies/pigeon-dir.yaml"),
        "description: d.\ntarget: { dir: infra/pigeon }\nrules:\n  - line: { at: 1, matches: '.' }\n",
    )
    .expect("write");
    let r = describe(root.path());
    let nodes = ids(&r, "nodes");
    assert!(
        nodes.contains(&"capability:cloudflare.r2.write".to_string()),
        "{nodes:?}"
    );
    assert!(!nodes.iter().any(|n| n == "capability:deploy"), "{nodes:?}");
    let edges = r.result["edges"].as_array().expect("edges");
    let has = |from: &str, to: &str| edges.iter().any(|e| e["from"] == from && e["to"] == to);
    assert!(has("unit:infra/pigeon", "capability:cloudflare.r2.write"));
    assert!(has("policy:pigeon-dir", "unit:infra/pigeon"));
    assert!(!has("policy:pigeon-dir", "repository"));
}

/// Each row of the link table is drawn only while its evidence is in the tree; a row whose code
/// is gone fails here, so the table follows the code.
#[test]
fn every_link_is_evidenced_by_the_tree() {
    let root = repo_root();
    let missing: Vec<&str> = agent::architecture::LINKS
        .iter()
        .filter(|r| !agent::architecture::evidenced(&root, r))
        .map(|r| r.id)
        .collect();
    assert!(missing.is_empty(), "no evidence for {missing:?}");
    let r = describe(&root);
    let network = &r.result["network"];
    let kept: Vec<&str> = network["links"]
        .as_array()
        .expect("links")
        .iter()
        .filter_map(|l| l["id"].as_str())
        .collect();
    for row in agent::architecture::LINKS {
        assert!(kept.contains(&row.id), "{} is not drawn", row.id);
    }
    let parts: Vec<&str> = network["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    for want in [
        "ChangeRun",
        "Instance",
        "MainRun",
        "Checker",
        "Runner",
        "D1",
        "R2",
        "Artifacts",
    ] {
        assert!(parts.contains(&want), "{want} is missing: {parts:?}");
    }
    assert!(network["worker"]["secrets"].as_u64().is_some_and(|n| n > 0));
    let text = network.to_string();
    for secret in [
        "AGENT_TOKEN",
        "SESSION_KEY",
        "APPROVER_TOTP",
        "GITHUB_APP_KEY",
    ] {
        assert!(!text.contains(secret), "{secret} reaches the network");
    }
}

#[test]
fn an_instance_without_the_worker_has_no_network() {
    let dir = TempDir::new().expect("tempdir");
    let config = dir.path().join(".config");
    fs::create_dir_all(config.join("policies")).expect("mkdir");
    fs::write(config.join("instance.yaml"), MANIFEST).expect("write");
    fs::write(config.join("providers.yaml"), "").expect("write");
    fs::write(config.join("releases.yaml"), "").expect("write");
    let r = describe(dir.path());
    assert!(r.result["network"].is_null(), "{}", r.result["network"]);
}

#[test]
fn a_row_whose_evidence_is_gone_is_not_evidenced() {
    use agent::architecture::{Row, evidenced};
    let dir = TempDir::new().expect("tempdir");
    let worker = dir.path().join("src/pigeon-agent/src/worker");
    fs::create_dir_all(&worker).expect("mkdir");
    fs::write(worker.join("merges.rs"), "Event::Behind; to_change(x)").expect("write");
    let row = |evidence| Row {
        id: "x",
        from: "a",
        to: "b",
        label: "",
        kind: agent::architecture::Kind::Call,
        evidence,
    };
    let ordered = row(&[
        ("worker/merges.rs", "Event::Behind"),
        ("worker/merges.rs", "to_change("),
    ]);
    assert!(evidenced(dir.path(), &ordered));
    let backwards = row(&[
        ("worker/merges.rs", "to_change("),
        ("worker/merges.rs", "Event::Behind"),
    ]);
    assert!(
        !evidenced(dir.path(), &backwards),
        "fragments are found in order"
    );
    fs::write(worker.join("merges.rs"), "to_change(x)").expect("write");
    assert!(!evidenced(dir.path(), &ordered));
}

/// The network's boxes never overlap, and each route leaves its source's border and ends on its
/// target's; see §8 and the 2026-11-11 amendment.
#[test]
fn the_network_diagram_is_placed_without_overlaps() {
    let r = describe(&repo_root());
    let d = &r.result["network"]["diagram"];
    let boxes = d["boxes"].as_array().expect("boxes");
    let rect = |b: &Value| {
        let n = |k: &str| b[k].as_i64().expect("number");
        (n("x"), n("y"), n("x") + n("w"), n("y") + n("h"))
    };
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            let (a, b) = (rect(a), rect(b));
            assert!(
                a.2 <= b.0 || b.2 <= a.0 || a.3 <= b.1 || b.3 <= a.1,
                "{a:?} overlaps {b:?}"
            );
        }
    }
    let at = |id: &str| {
        rect(
            boxes
                .iter()
                .find(|b| b["id"] == id)
                .unwrap_or_else(|| panic!("{id} has no box")),
        )
    };
    let links = r.result["network"]["links"].as_array().expect("links");
    for route in d["routes"].as_array().expect("routes") {
        let link = links
            .iter()
            .find(|l| l["id"] == route["id"])
            .expect("a route is a link's");
        let path = route["d"].as_str().expect("d");
        let numbers: Vec<i64> = path
            .split(|c: char| c.is_ascii_alphabetic() || c == ' ')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().expect("integer"))
            .collect();
        let (s, t) = (
            at(link["from"].as_str().expect("from")),
            at(link["to"].as_str().expect("to")),
        );
        let (sx, sy) = (numbers[0], numbers[1]);
        assert!(
            (s.0..=s.2).contains(&sx) && (s.1..=s.3).contains(&sy),
            "{path} leaves {s:?}"
        );
        let last = *numbers.last().expect("end");
        let ends = path
            .rsplit(|c: char| c.is_ascii_digit() || c == '-')
            .find(|s| !s.is_empty());
        let near = |v: i64, edge: i64| (v - edge).abs() <= 1;
        let ok = match ends {
            Some("V") => near(last, t.1) || near(last, t.3),
            Some("H") => near(last, t.0) || near(last, t.2),
            _ => false,
        };
        assert!(ok || link["from"] == link["to"], "{path} ends off {t:?}");
    }
}

/// The paths `text` declares in route tables, `Path::Exact("…")` and `Path::Prefix("…")`.
fn table_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for kind in ["Path::Exact(\"", "Path::Prefix(\""] {
        for s in text.split(kind).skip(1) {
            if let Some(end) = s.find('"') {
                out.push(s[..end].to_string());
            }
        }
    }
    out
}

/// Every route the Worker serves, from the platform's own sources: each surface's table, the
/// literals and constants in `fetch`'s body, and each app API literal.
fn served(root: &std::path::Path) -> Vec<String> {
    let worker = root.join("src/pigeon-agent/src/worker");
    let src = root.join("src/pigeon-agent/src");
    let mut out = Vec::new();
    let mut files: Vec<_> = fs::read_dir(&worker)
        .expect("worker sources")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    for f in &files {
        let text = fs::read_to_string(f).expect("read");
        let text = text.split("#[cfg(test)]").next().unwrap_or("");
        out.extend(table_paths(text));
        out.extend(agent::architecture::literals(text, "/v0/app/"));
    }
    let main = fs::read_to_string(worker.join("mod.rs")).expect("read");
    let body = &main[main.find("async fn fetch(").expect("fetch")..];
    let body = &body[..body.find("\n}\n").expect("fetch's end")];
    out.extend(agent::architecture::literals(body, "/"));
    // `module::NAME` constants, resolved in `worker/<module>.rs` or `<module>.rs`.
    for piece in body.split("::").skip(1) {
        let name: String = piece
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || *c == '_')
            .collect();
        if name.len() < 2 || piece[name.len()..].starts_with(['(', ':']) {
            continue;
        }
        let decl = format!("const {name}: &str = \"");
        let value = files
            .iter()
            .chain(
                fs::read_dir(&src)
                    .expect("src")
                    .flatten()
                    .map(|e| e.path())
                    .collect::<Vec<_>>()
                    .iter(),
            )
            .filter_map(|f| fs::read_to_string(f).ok())
            .find_map(|t| {
                let at = t.find(&decl)? + decl.len();
                Some(t[at..at + t[at..].find('"')?].to_string())
            });
        if let Some(v) = value.filter(|v| v.starts_with('/')) {
            out.push(v);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The coverage check: every route, binding, hostname and outbound host of the Worker is in
/// the network; see `.claude/decisions/2026-11-15-runtime-architecture-coverage-agents-deploys.md` §2.
#[test]
fn every_route_binding_and_outbound_host_is_in_the_network() {
    let root = repo_root();
    let r = describe(&root);
    let net = &r.result["network"];
    let mut gaps = route_gaps(&root, net);
    gaps.extend(spec_gaps(&root, net));
    gaps.extend(outbound_gaps(&root, net));
    assert!(gaps.is_empty(), "the network lacks: {gaps:#?}");
}

/// Each served path is a client's item, under a listed prefix, or beside an item.
fn route_gaps(root: &std::path::Path, net: &Value) -> Vec<String> {
    let mut gaps = Vec::new();
    let strip = |p: &str| p.rsplit(' ').next().unwrap_or(p).to_string();
    let mut listed: Vec<String> = Vec::new();
    for c in net["clients"].as_array().expect("clients") {
        for it in c["items"].as_array().expect("items") {
            listed.push(strip(it["k"].as_str().expect("k")));
            listed.extend(
                it["v"]
                    .as_str()
                    .unwrap_or("")
                    .split(' ')
                    .filter(|t| t.starts_with('/'))
                    .map(str::to_string),
            );
        }
    }
    for path in served(root) {
        let covered = listed.iter().any(|l| match l.strip_suffix('…') {
            Some(prefix) => path.starts_with(prefix),
            None => *l == path || (l.ends_with('/') && path.starts_with(l.as_str())),
        });
        if !covered {
            gaps.push(format!("route {path}"));
        }
    }
    gaps
}

/// Every field of the Worker's spec is known, and each holds what the network shows.
fn spec_gaps(root: &std::path::Path, net: &Value) -> Vec<String> {
    let mut gaps = Vec::new();
    let found = agent::workers::discover(root)
        .expect("discovers")
        .into_iter()
        .find(|f| f.spec.entry.krate.ends_with("pigeon-agent"))
        .expect("the control plane's Worker");
    let spec = &found.spec;
    let w = &net["worker"];
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    };
    let bindings = serde_json::to_value(&spec.bindings).expect("json");
    // `vars` are settings, and `version_metadata` names the running version: not parts.
    for k in bindings.as_object().expect("object").keys() {
        if ![
            "d1",
            "r2",
            "secrets_store",
            "queues",
            "vars",
            "version_metadata",
        ]
        .contains(&k.as_str())
        {
            gaps.push(format!("binding kind {k}"));
        }
    }
    for (kind, have) in [
        ("d1", spec.bindings.d1.keys().cloned().collect::<Vec<_>>()),
        ("r2", spec.bindings.r2.keys().cloned().collect()),
        (
            "secrets_store",
            spec.bindings.secrets_store.keys().cloned().collect(),
        ),
    ] {
        for b in have {
            if !names(&w[kind]).contains(&b) {
                gaps.push(format!("{kind} binding {b}"));
            }
        }
    }
    if w["queues"].as_u64() != Some(spec.bindings.queues.consumers.len() as u64) {
        gaps.push("queue consumers".into());
    }
    if names(&w["crons"]) != spec.crons {
        gaps.push("crons".into());
    }
    if w["secrets"].as_u64() != Some(spec.secrets.len() as u64) {
        gaps.push("secrets' count".into());
    }
    let containers: Vec<String> = w["containers"]
        .as_array()
        .expect("containers")
        .iter()
        .filter_map(|c| c["class"].as_str().map(String::from))
        .collect();
    for class in &spec.durable_objects.classes {
        if !names(&w["objects"]).contains(class) && !containers.contains(class) {
            gaps.push(format!("Durable Object {class}"));
        }
    }
    for route in &spec.routes {
        for k in serde_json::to_value(route)
            .expect("json")
            .as_object()
            .expect("object")
            .keys()
        {
            if !agent::architecture::ROUTE_KEYS.contains(&k.as_str()) {
                gaps.push(format!("route kind {k}"));
            }
        }
    }
    if w["hosts"].as_array().map(Vec::len) != Some(spec.routes.len()) {
        gaps.push("the Worker's hostnames".into());
    }
    gaps
}

/// Each outbound host is the platform's own, a test's, or a service's; each link ends at parts.
fn outbound_gaps(root: &std::path::Path, net: &Value) -> Vec<String> {
    let mut gaps = Vec::new();
    for file in agent::architecture::outbound_files(root) {
        let text = fs::read_to_string(&file).expect("read");
        for host in agent::architecture::hosts_in(&text) {
            let known = agent::architecture::internal(&host)
                || agent::architecture::SERVICES
                    .iter()
                    .any(|s| s.hosts.contains(&host.as_str()));
            if !known {
                gaps.push(format!("host {host} in {}", file.display()));
            }
        }
    }

    // Links: both ends are parts.
    let parts: Vec<String> = net["parts"]
        .as_array()
        .expect("parts")
        .iter()
        .filter_map(|p| p["id"].as_str().map(String::from))
        .collect();
    for l in net["links"].as_array().expect("links") {
        for end in ["from", "to"] {
            let id = l[end].as_str().unwrap_or("");
            if id != "Worker" && !parts.iter().any(|p| p == id) {
                gaps.push(format!("link {} ends at {id}", l["id"]));
            }
        }
    }
    for want in ["Secrets Store", "Cloudflare API", "APNs", "Web Push"] {
        if !parts.iter().any(|p| p == want) {
            gaps.push(format!("part {want}"));
        }
    }
    gaps
}

/// The graph names no zone but `pigeon.dev`'s, no checkout path, no 32-hex ID and no
/// standalone's hostname; see the same ADR §2.
#[test]
fn the_graph_names_only_pigeon_dev() {
    let r = describe(&repo_root());
    let text = serde_json::to_string(&r.result).expect("json");
    assert!(
        !text.contains("noisypigeon.com"),
        "a zone other than pigeon.dev"
    );
    assert!(
        !text.contains(
            &repo_root()
                .canonicalize()
                .expect("root")
                .display()
                .to_string()
        ),
        "a checkout's path"
    );
    let hex32 = text
        .split(|c: char| !c.is_ascii_hexdigit())
        .any(|t| t.len() == 32);
    assert!(!hex32, "a 32-hex ID");
    for h in r.result["network"]["access"]["hosts"]
        .as_array()
        .expect("hosts")
        .iter()
        .chain(
            r.result["network"]["worker"]["hosts"]
                .as_array()
                .expect("hosts")
                .iter()
                .map(|h| &h["k"]),
        )
    {
        let h = h.as_str().expect("host");
        assert!(
            h.ends_with("pigeon.dev") || h == agent::architecture::STANDALONE,
            "{h}"
        );
    }
}

#[test]
fn the_network_renders_as_text_and_svg() {
    let r = describe(&repo_root());
    let text = agent::architecture::render::text(&r.result);
    for want in ["clients", "iOS app", "GRANTER", "APNs", "-> Secrets Store"] {
        assert!(text.contains(want), "{want} is missing from the text");
    }
    let svg = agent::architecture::render::svg(&r.result);
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.contains("Cloudflare API") && svg.trim_end().ends_with("</svg>"));
    assert!(
        agent::architecture::render::formatted(r.result.clone(), Some("dot")).is_err(),
        "an unknown format is refused"
    );
}

#[test]
fn the_worker_reads_the_signing_key_only_under_its_stored_name() {
    let dir = repo_root().join("src/pigeon-agent/src/worker");
    for entry in std::fs::read_dir(&dir).expect("worker sources") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source");
        assert!(
            !text.contains("worker::TOKEN_VAR"),
            "{}: the Worker's secret is read through checker::signing_key",
            path.display()
        );
    }
}
