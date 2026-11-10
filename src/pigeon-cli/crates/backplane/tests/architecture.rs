//! `architecture.describe` derives the graph from the tree; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §2.

use std::fs;
use std::path::PathBuf;

use backplane::{Instance, Request, Response, run_with};

mod common;
use serde_json::Value;
use tempfile::TempDir;

const MANIFEST: &str = "description: A test instance.\nschema: v0\nname: test\nrepository: https://example.invalid/test\npaths:\n  policies: .config/policies\n  providers: .config/providers.yaml\n  releases: .config/releases.yaml\n";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
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
        "policy:backplane",
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
    s["x-cobot"]["credentials"] = serde_json::json!({
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
