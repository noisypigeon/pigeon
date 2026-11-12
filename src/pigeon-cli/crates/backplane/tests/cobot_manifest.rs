//! An instance's manifest v1 of kind-typed units; see
//! `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use tempfile::TempDir;

const ZONE: &str = "noisypigeon.com";
const STATE_PASS: &str = "state-passphrase-0123456789";
const TOKEN: &str = "run-token-not-in-any-file";
const PROTONMAIL: &str = "e9457a054010bbbb12cc4abfce39d67198e9abd1";
const PM: &str = "yulfrkphygclvbkaipymgmyhstrxuj";

const MANIFEST: &str = "schema: v1
name: noisypigeon
description: Willow Graysen's sites.
repository: https://github.com/noisypigeon/noisypigeon
units:
  zones/noisypigeon.com:
    kind: cloudflare/zone
    services:
      proton:
        verification: { protonmail: PROTONMAIL, pm: PM }
        dmarc: default
    records: {}
  sites/blog:
    kind: cloudflare/static-site
    zone: noisypigeon.com
    hosts: [\"@\", www]
    template: { name: blog, version: 1.0.2, features: [math] }
    content: sites/blog
    site: { title: Willow Graysen }
moves:
  - { from: workloads/noisypigeon.com/apex, to: zones/noisypigeon.com }
  - { from: workloads/noisypigeon.com/blog, to: sites/blog }
";

const APEX: &str = "apex: blog
services:
  proton:
    verification: { protonmail: PROTONMAIL, pm: PM }
    dmarc: default
records: {}
";

const BLOG: &str = "static_site:
  template: { name: blog, version: 1.0.2, features: [math] }
  site: { title: Willow Graysen }
";

fn manifest() -> String {
    MANIFEST
        .replace("PROTONMAIL", PROTONMAIL)
        .replace("PM,", &format!("{PM},"))
        .replace(": PM }", &format!(": {PM} }}"))
}

fn apex() -> String {
    APEX.replace("PROTONMAIL", PROTONMAIL)
        .replace(": PM }", &format!(": {PM} }}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn copy_from(from: &Path, root: &Path, rel: &str) {
    let src = from.join(rel);
    if src.is_dir() {
        for e in fs::read_dir(&src).expect("list").flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name != "CHANGELOG.md" {
                copy_from(from, root, &format!("{rel}/{name}"));
            }
        }
    } else {
        write(root, rel, &fs::read_to_string(src).expect("read"));
    }
}

/// The real Cloudflare catalog, with its kinds, and providers.
fn platform() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for rel in [
        ".config/providers.yaml",
        ".config/providers.lock.hcl",
        "catalog/cloudflare",
        "catalog/agnostic",
        "catalog/pigeon",
    ] {
        copy_from(&repo_root(), dir.path(), rel);
    }
    dir
}

/// An instance holding only `config.yaml` and the blog's content.
fn instance(manifest: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    write(dir.path(), "config.yaml", manifest);
    write(
        dir.path(),
        "sites/blog/posts/2026-11-05-hello.md",
        "+++\n+++\n",
    );
    dir
}

fn problems(root: &Path, platform: &Path) -> Vec<String> {
    let allow = backplane::cobot::providers::Allowlist::load(&platform.join(".config"))
        .expect("allowlist loads");
    backplane::cobot::unit::validate_in(root, platform, &allow)
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

fn render(root: &Path, platform: &Path, unit: &str) -> Value {
    let (units, problems) = backplane::cobot::unit::discover_in(root, platform);
    assert!(problems.is_empty(), "{problems:?}");
    let unit = units
        .iter()
        .find(|u| u.path == unit)
        .unwrap_or_else(|| panic!("no unit {unit}"));
    let allow =
        backplane::cobot::providers::Allowlist::load(&platform.join(".config")).expect("allowlist");
    backplane::cobot::render_json(
        unit,
        Path::new("/state"),
        &allow,
        0,
        &backplane::cobot::config::Existing::new(),
    )
}

fn items(rendered: &Value, block: &str) -> Map<String, Value> {
    rendered["module"][block]["for_each"]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_manifest_declares_units_by_kind() {
    let platform = platform();
    let inst = instance(&manifest());
    let (units, found) = backplane::cobot::unit::discover_in(inst.path(), platform.path());
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(
        units
            .iter()
            .map(|u| (u.path.as_str(), u.kind.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("sites/blog", Some("cloudflare/static-site")),
            ("zones/noisypigeon.com", Some("cloudflare/zone")),
        ]
    );
    let got = problems(inst.path(), platform.path());
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_bad_manifest_names_what_is_wrong() {
    let platform = platform();
    for (from, to, want) in [
        (
            "kind: cloudflare/zone",
            "kind: cloudflare/nope",
            "no kind `cloudflare/nope`",
        ),
        ("sites/blog:\n", "sites/../blog:\n", "is not a unit ID"),
        (
            "content: sites/blog",
            "content: ../elsewhere",
            "`content` is a relative path",
        ),
        (
            "zone: noisypigeon.com\n    hosts",
            "zone: other.com\n    hosts",
            "no zone unit",
        ),
        ("hosts: [\"@\", www]", "hosts: []", "units.sites/blog"),
        (
            "records: {}",
            "records: {}\n    apex: blog",
            "units.zones/noisypigeon.com",
        ),
        (
            "description: Willow Graysen's sites.",
            "description: \"\"",
            "`description` must not be empty",
        ),
        ("schema: v1", "schema: v2", "is manifest `schema: v1`"),
        (
            "  sites/blog:\n",
            "  zones/again/noisypigeon.com:\n    kind: cloudflare/zone\n    services: {}\n    records: {}\n  sites/blog:\n",
            "declares zone noisypigeon.com again",
        ),
        (
            "sites/blog:\n",
            "sites/a-very-long-site-name-that-with-its-zone-overflows-r2:\n",
            "longer than R2's 63 characters",
        ),
    ] {
        let text = manifest().replace(from, to);
        assert_ne!(text, manifest(), "{from}");
        let inst = instance(&text);
        let got = problems(inst.path(), platform.path());
        assert!(
            got.iter().any(|p| p.contains(want)),
            "want {want:?}: {got:?}"
        );
    }
}

#[test]
fn two_sites_on_one_bucket_are_refused() {
    let platform = platform();
    let text = manifest().replace(
        "moves:",
        "  zones/a-b.example:
    kind: cloudflare/zone
    services: {}
    records: {}
  zones/a.b-example:
    kind: cloudflare/zone
    services: {}
    records: {}
  sites/one/blog:
    kind: cloudflare/static-site
    zone: a-b.example
    hosts: [\"@\"]
    template: { name: blog, version: 1.0.2 }
    site: {}
  sites/two/blog:
    kind: cloudflare/static-site
    zone: a.b-example
    hosts: [\"@\"]
    template: { name: blog, version: 1.0.2 }
    site: {}
moves:",
    );
    let inst = instance(&text);
    let got = problems(inst.path(), platform.path());
    assert!(
        got.iter()
            .any(|p| p.contains("bucket `blog-a-b-example` is also sites/one/blog's")),
        "{got:?}"
    );
}

#[cfg(unix)]
#[test]
fn content_through_a_symlink_is_refused() {
    let platform = platform();
    let outside = TempDir::new().expect("outside");
    write(outside.path(), "posts/secret.md", "not for the site");
    let inst = instance(&manifest().replace("content: sites/blog", "content: sites/linked"));
    std::os::unix::fs::symlink(outside.path(), inst.path().join("sites/linked")).expect("link");
    let got = problems(inst.path(), platform.path());
    assert!(
        got.iter().any(|p| p.contains("through no symlink")),
        "{got:?}"
    );
    assert!(
        backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).is_err(),
        "an invalid instance builds nothing"
    );
}

#[test]
fn a_stored_plan_sees_a_change_to_its_units_settings() {
    let platform = platform();
    let inst = instance(&manifest());
    let state = TempDir::new().expect("state");
    let digest = |root: &Path| {
        let (units, _) = backplane::cobot::unit::discover_in(root, platform.path());
        units
            .iter()
            .map(|u| {
                let work = backplane::cobot::render(
                    root,
                    &platform.path().join(".config"),
                    u,
                    state.path(),
                )
                .expect("render");
                backplane::cobot::plans::digest(u, &work).expect("digest")
            })
            .collect::<Vec<_>>()
    };
    let before = digest(inst.path());
    write(
        inst.path(),
        "config.yaml",
        &manifest().replace("hosts: [\"@\", www]", "hosts: [\"@\"]"),
    );
    let after = digest(inst.path());
    assert_ne!(before[0], after[0], "the site's own hosts changed");
    assert_ne!(before[1], after[1], "the zone's rules follow its sites");
}

#[test]
fn a_host_claimed_by_a_site_and_a_record_is_refused() {
    let platform = platform();
    let text = manifest().replace(
        "records: {}",
        "records:\n      web: { name: www, type: A, content: 192.0.2.1 }",
    );
    let inst = instance(&text);
    let got = problems(inst.path(), platform.path());
    assert!(
        got.iter()
            .any(|p| p.contains("www ADDRESS is claimed by workload blog and records.web")),
        "{got:?}"
    );
    let two = manifest().replace(
        "moves:",
        "  sites/blog2:
    kind: cloudflare/static-site
    zone: noisypigeon.com
    hosts: [www]
    template: { name: blog, version: 1.0.2 }
    site: {}
moves:",
    );
    let inst = instance(&two);
    let got = problems(inst.path(), platform.path());
    assert!(
        got.iter()
            .any(|p| p.contains("www ADDRESS is claimed by workload blog and workload blog2")),
        "{got:?}"
    );
}

/// Today's apex and blog, as `workloads/` units of an instance.
fn old_instance() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    write(
        dir.path(),
        &format!("workloads/{ZONE}/apex/config.yaml"),
        &apex(),
    );
    write(
        dir.path(),
        &format!("workloads/{ZONE}/blog/config.yaml"),
        BLOG,
    );
    dir
}

#[test]
fn moved_units_keep_every_address_and_item() {
    let platform = platform();
    let (old, new) = (old_instance(), instance(&manifest()));
    let (p, o, n) = (platform.path(), old.path(), new.path());
    let old_apex = render(o, p, &format!("workloads/{ZONE}/apex"));
    let zone = render(n, p, &format!("zones/{ZONE}"));
    for block in ["dns_records", "zone_rulesets"] {
        assert_eq!(items(&old_apex, block), items(&zone, block), "{block}");
    }
    assert_eq!(items(&zone, "dns_records").len(), 9);
    assert_eq!(items(&zone, "zone_rulesets").len(), 2);
    let old_blog = render(o, p, &format!("workloads/{ZONE}/blog"));
    let site = render(n, p, "sites/blog");
    for block in ["bucket", "domains"] {
        assert_eq!(items(&old_blog, block), items(&site, block), "{block}");
    }
    assert_eq!(
        items(&site, "domains").keys().collect::<Vec<_>>(),
        ["apex", "www"]
    );
    for rendered in [&zone, &site] {
        assert!(rendered.get("provider").is_some());
        assert_eq!(
            rendered["provider"]["cloudflare"]["api_token"],
            "${var.credentials[\"cloudflare\"].api_token}"
        );
        assert!(rendered["variable"].get("handoff").is_none());
    }
}

#[test]
fn a_site_unit_builds_from_its_content_at_its_own_hosts() {
    let platform = platform();
    let inst = instance(&manifest());
    let sites = backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect("found");
    assert_eq!(sites.len(), 1);
    let blog = &sites[0];
    assert_eq!(
        (
            blog.unit.as_str(),
            blog.workload.as_str(),
            blog.template.as_str()
        ),
        ("sites/blog", "blog", "blog")
    );
    assert_eq!(
        blog.content,
        inst.path()
            .canonicalize()
            .expect("canonical")
            .join("sites/blog")
    );
    assert_eq!(blog.hosts, [ZONE.to_string(), format!("www.{ZONE}")]);
    assert_eq!(blog.bucket(), "blog-noisypigeon-com");
    assert_eq!(blog.settings, Some(json!({ "title": "Willow Graysen" })));
}

/// The blog served by a platform unit at `/docs`, with no host of its own.
fn served_manifest() -> String {
    manifest()
        .replace(
            "hosts: [\"@\", www]",
            "hosts: []\n    served_by: { unit: platform/blue, path: /docs }",
        )
        .replace(
            "moves:",
            "  platform/blue:
    kind: pigeon/platform
    worker: w
    hostname: w.example.workers.dev
    domains: [platform.example.dev, blue.platform.example.dev]
    instance: i
    change_floor: 1
    database: { name: d }
    buckets: [ { name: b } ]
    queue: { name: q }
    access: { application: a, owners_policy: o }
    service_tokens: []
moves:",
        )
}

#[test]
fn a_served_site_builds_at_its_platforms_path_and_claims_no_domain() {
    let platform = platform();
    let inst = instance(&served_manifest());
    let got = problems(inst.path(), platform.path());
    assert!(got.is_empty(), "{got:?}");
    let sites = backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect("found");
    let blog = &sites[0];
    assert!(blog.hosts.is_empty(), "{:?}", blog.hosts);
    assert_eq!(
        blog.url().as_deref(),
        Some("https://platform.example.dev/docs")
    );
    let rendered = render(inst.path(), platform.path(), "sites/blog");
    assert!(items(&rendered, "domains").is_empty(), "{rendered}");
    assert_eq!(items(&rendered, "bucket").len(), 1, "{rendered}");
}

#[test]
fn a_site_served_by_no_platform_is_refused() {
    let platform = platform();
    let inst = instance(&served_manifest().replace("unit: platform/blue", "unit: platform/nope"));
    let err =
        backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect_err("refused");
    assert!(err.to_string().contains("served_by"), "{err}");
}

#[test]
fn the_runs_token_fills_each_cloudflare_credential_it_is_not_given() {
    let providers = BTreeMap::from([
        ("cloudflare".to_string(), "cloudflare".to_string()),
        ("cloudflare_zone".to_string(), "cloudflare".to_string()),
        ("store".to_string(), "backblaze".to_string()),
    ]);
    let filled: Value = serde_json::from_str(
        &backplane::cobot::fill_credentials(
            Some(r#"{"cloudflare_zone":{"api_token":"minted"}}"#),
            &providers,
            Some(TOKEN),
        )
        .expect("filled"),
    )
    .expect("json");
    assert_eq!(
        filled,
        json!({
            "cloudflare": { "api_token": TOKEN },
            "cloudflare_zone": { "api_token": "minted" }
        })
    );
    assert_eq!(
        backplane::cobot::fill_credentials(None, &providers, None),
        None
    );
    assert_eq!(
        backplane::cobot::fill_credentials(Some("{}"), &providers, Some("")).as_deref(),
        Some("{}")
    );
}

#[test]
fn a_credentials_target_is_a_name_or_a_config_path() {
    let platform = platform();
    let schema = platform
        .path()
        .join("catalog/cloudflare/kinds/zone/schema.json");
    let text = fs::read_to_string(&schema).expect("read");
    fs::write(
        &schema,
        text.replace("\"target\": \"$config.zone\"", "\"target\": \"$secret.x\""),
    )
    .expect("write");
    let inst = instance(&manifest());
    let got = problems(inst.path(), platform.path());
    assert!(
        got.iter()
            .any(|p| p.contains("`target` is a name or `$config.<path>`")),
        "{got:?}"
    );
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn data_module(root: &Path, rel: &str, vars: &[&str], output: &str) {
    let mut tf = String::new();
    for v in vars {
        let _ = write!(
            tf,
            "variable \"{v}\" {{\n  type    = any\n  default = null\n}}\n"
        );
    }
    let input = vars
        .iter()
        .map(|v| format!("{v} = var.{v}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        tf,
        "resource \"terraform_data\" \"this\" {{\n  input = {{ {input} }}\n}}\n\noutput \"id\" {{\n  value = terraform_data.this.id\n}}\n{output}"
    );
    write(root, &format!("{rel}/main.tf"), &tf);
    write(
        root,
        &format!("{rel}/module.yaml"),
        "version: 0.1.0\ndescription: A test module.\n",
    );
}

/// A platform whose Cloudflare modules are built-in stand-ins, so `tofu` needs no network; its
/// lookups' IDs are sensitive, as the handoff's were, so a move changes no input's sensitivity.
fn offline_platform() -> TempDir {
    let dir = platform();
    let root = dir.path();
    let modules = root.join("catalog/cloudflare/modules");
    fs::remove_dir_all(&modules).expect("clear modules");
    let schema = root.join("catalog/cloudflare/schema.json");
    let mut s: Value =
        serde_json::from_str(&fs::read_to_string(&schema).expect("read")).expect("json");
    for kind in ["apex", "workload"] {
        s["x-cobot"][kind]["providers"] = json!({});
    }
    fs::write(&schema, s.to_string()).expect("write");
    for kind in ["zone", "static-site"] {
        let path = root.join(format!("catalog/cloudflare/kinds/{kind}/schema.json"));
        let mut k: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        k["x-cobot"]["providers"] = json!({});
        fs::write(&path, k.to_string()).expect("write");
    }
    let named = "output \"name\" {\n  value = terraform_data.this.id\n}\n";
    for (module, vars, output) in [
        ("ruleset", &["zone_id", "name", "phase", "rules"][..], named),
        (
            "r2-bucket",
            &["account_id", "name", "storage_class"][..],
            named,
        ),
        (
            "r2-custom-domain",
            &["account_id", "bucket_name", "domain", "zone_id"][..],
            named,
        ),
        (
            "dns-record",
            &[
                "zone_id", "name", "type", "content", "ttl", "priority", "proxied",
            ][..],
            "",
        ),
    ] {
        data_module(
            root,
            &format!("catalog/cloudflare/modules/{module}"),
            vars,
            output,
        );
    }
    write(
        root,
        "catalog/cloudflare/modules/account/main.tf",
        "resource \"terraform_data\" \"this\" {}\noutput \"id\" {\n  value     = \"acct-1\"\n  sensitive = true\n}\n",
    );
    write(
        root,
        "catalog/cloudflare/modules/zone/main.tf",
        "variable \"name\" {\n  type = string\n}\nvariable \"account_id\" {\n  type = string\n}\nresource \"terraform_data\" \"this\" {}\noutput \"id\" {\n  value     = \"zone-np\"\n  sensitive = true\n}\n",
    );
    for module in ["account", "zone"] {
        write(
            root,
            &format!("catalog/cloudflare/modules/{module}/module.yaml"),
            "version: 0.1.0\ndescription: A test lookup.\n",
        );
    }
    dir
}

/// A fake Cloudflare API that sees one account and `noisypigeon.com`.
fn fake_api() -> String {
    use std::io::{BufRead, BufReader, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim_end().is_empty() {
                    break;
                }
            }
            let body = if line.contains("/zones?name=noisypigeon.com") {
                json!({ "success": true, "result": [{ "id": "zone-np", "name": ZONE }] })
            } else if line.contains("/accounts?") {
                json!({ "success": true, "result": [{ "id": "acct-1" }] })
            } else {
                json!({ "success": false })
            };
            let text = body.to_string();
            let mut out = stream;
            let _ = write!(
                out,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    format!("http://{addr}")
}

fn ctx(root: &Path, platform: &Path, state: &Path, api: &str) -> backplane::cobot::Context {
    backplane::cobot::Context {
        root: root.to_path_buf(),
        providers: platform.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: STATE_PASS.into(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: backplane::cobot::generations::day("2026-11-05").expect("day"),
        credentials: None,
        cloudflare_token: Some(TOKEN.into()),
        cloudflare_api: Some(api.into()),
        platform: Some(platform.to_path_buf()),
        adopt: false,
        provider_config: None,
        provider_token: false,
    }
}

fn grep(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with(".terraform") {
                    stack.push(p);
                }
            } else if fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle)) {
                hits.push(p);
            }
        }
    }
    hits
}

#[test]
fn the_first_plan_of_moved_units_destroys_and_replaces_nothing() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let platform = offline_platform();
    let p = platform.path();
    let state = TempDir::new().expect("state");
    let api = fake_api();

    let old = old_instance();
    let units = backplane::cobot::select_in(old.path(), p, &p.join(".config"), &[]).expect("valid");
    let applied =
        backplane::cobot::apply(&ctx(old.path(), p, state.path(), &api), &units).expect("apply");
    assert_eq!(
        applied
            .iter()
            .map(backplane::cobot::UnitPlan::counts)
            .collect::<Vec<_>>(),
        [(11, 0, 0), (3, 0, 0)],
        "{}",
        backplane::cobot::summary(&applied)
    );

    let new = instance(&manifest());
    let units = backplane::cobot::select_in(new.path(), p, &p.join(".config"), &[]).expect("valid");
    let run = ctx(new.path(), p, state.path(), &api);
    let plans = backplane::cobot::plan(&run, &units).expect("plan");
    let summary = backplane::cobot::summary(&plans);
    for plan in &plans {
        assert_eq!(
            plan.counts(),
            (2, 0, 0),
            "only the stand-in lookups: {summary}"
        );
        assert!(
            plan.note
                .as_deref()
                .is_some_and(|n| n.contains("State moves from `workloads/")),
            "{summary}"
        );
    }
    let applied = backplane::cobot::apply(&run, &units).expect("apply moved");
    assert!(
        applied.iter().all(|p| p.counts() == (2, 0, 0)),
        "{}",
        backplane::cobot::summary(&applied)
    );
    assert!(
        !state
            .path()
            .join("workloads")
            .join(ZONE)
            .join("apex/terraform.tfstate")
            .exists()
    );
    for dir in [new.path().join(".cobot"), state.path().to_path_buf()] {
        assert_eq!(grep(&dir, TOKEN), Vec::<PathBuf>::new());
    }
}

/// While releases pause, a site may read its template from its own instance's tree; see the
/// 2026-11-11 amendment of
/// `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md`.
#[test]
fn a_tree_pin_reads_head_only_for_a_template_in_the_instances_own_tree() {
    let platform = platform();
    let pinned = manifest().replace("version: 1.0.2", "version: tree");
    let inst = instance(&pinned);
    let err =
        backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect_err("refused");
    assert!(err.to_string().contains("own tree"), "{err}");

    let template = inst.path().join("src/site-templates/blog");
    fs::create_dir_all(&template).expect("mkdir");
    fs::write(template.join("template.yaml"), "version: 0.1.0\n").expect("write");
    let sites = backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect("found");
    assert_eq!(
        (sites[0].revision(), sites[0].tag()),
        ("HEAD".to_string(), "site-template-blog@tree".to_string())
    );
}

#[test]
fn a_released_pin_still_reads_its_tag() {
    let platform = platform();
    let inst = instance(&manifest());
    let sites = backplane::cobot::sites::find_in(inst.path(), platform.path(), &[]).expect("found");
    assert_eq!(sites[0].revision(), "site-template-blog/v1.0.2");
}
