//! Worker deploys from the pipeline: the outputs snapshot, images by reference, versions and
//! health; see `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`.

use std::collections::BTreeMap;
use std::fs;

use agent::workers;
use agent::workers::ship::{self, Images};
use serde_json::{Map, Value, json};
use tempfile::TempDir;

const SPEC: &str = r#"description: The platform's control plane.
name: pigeon-control
entry: { crate: crates/agent, features: [worker] }
compatibility_date: "2026-10-01"
durable_objects:
  classes: [Runner]
  migrations: [ { tag: v1, new_sqlite_classes: [Runner] } ]
containers:
  - { class: Runner, image: { dockerfile: Dockerfile, target: ci }, instance_type: standard-1, max_instances: 4 }
bindings:
  d1: { DB: { name: pigeon, id: $output.infra/pigeon.database_id, migrations: crates/agent/migrations } }
  r2: { LOGS: "$output.infra/pigeon.bucket_names[pigeon-ci]" }
  vars: { PUBLIC_HOST: control.example.workers.dev, STATE_BUCKET: "$output.infra/pigeon.bucket_names[pigeon-state]" }
  version_metadata: CF_VERSION_METADATA
"#;

const DB: &str = "fedcba9876543210fedcba9876543210";
const IMAGE: &str = "registry.cloudflare.com/0123456789abcdef/pigeon-control-runner:abc";

fn fixture() -> (TempDir, workers::Found) {
    fixture_with(SPEC)
}

fn fixture_with(spec: &str) -> (TempDir, workers::Found) {
    let root = TempDir::new().expect("tempdir");
    let p = root.path();
    for d in ["infra/pigeon/workers/control", "crates/agent/migrations"] {
        fs::create_dir_all(p.join(d)).expect("mkdir");
    }
    fs::write(p.join("crates/agent/Cargo.toml"), "[package]\n").expect("write");
    fs::write(p.join("Dockerfile"), "FROM scratch AS ci\n").expect("write");
    fs::write(p.join("infra/pigeon/workers/control/worker.yaml"), spec).expect("write");
    let found = workers::discover(p)
        .expect("discovers")
        .into_iter()
        .next()
        .expect("one");
    assert_eq!(workers::validate(p, &found), Vec::new());
    (root, found)
}

fn outputs() -> BTreeMap<String, Map<String, Value>> {
    let mut m = Map::new();
    m.insert("database_id".into(), json!(DB));
    m.insert(
        "bucket_names".into(),
        json!(["pigeon-ci-x1", "pigeon-state-x2"]),
    );
    m.insert("zone_ids".into(), json!(["not referenced"]));
    let mut other = Map::new();
    other.insert("secretish".into(), json!("never referenced"));
    BTreeMap::from([
        ("infra/pigeon".to_string(), m),
        ("infra/other".to_string(), other),
    ])
}

#[test]
fn the_snapshot_holds_only_what_a_spec_references() {
    let (_root, found) = fixture();
    let snap = ship::snapshot(&[found.spec], &outputs());
    assert_eq!(snap.len(), 1, "only infra/pigeon");
    let pigeon = &snap["infra/pigeon"];
    let mut keys: Vec<&String> = pigeon.keys().collect();
    keys.sort();
    assert_eq!(keys, ["bucket_names", "database_id"]);
}

#[test]
fn images_are_rendered_by_reference_and_a_missing_one_is_named() {
    let (_root, found) = fixture();
    let images = BTreeMap::from([("pigeon-control/Runner".to_string(), IMAGE.to_string())]);
    let v = workers::render_with(&found, &outputs(), Some(&images)).expect("renders");
    assert_eq!(v["containers"][0]["image"], IMAGE);
    assert_eq!(
        v["version_metadata"],
        json!({ "binding": "CF_VERSION_METADATA" })
    );
    assert_eq!(v["d1_databases"][0]["database_id"], DB);
    let local = workers::render(&found, &outputs()).expect("renders");
    assert_eq!(local["containers"][0]["image"], "./Runner.Dockerfile");
    let problems =
        workers::render_with(&found, &outputs(), Some(&BTreeMap::new())).expect_err("no image");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("pigeon-control/Runner") && p.contains("images push")),
        "{problems:?}"
    );
}

#[test]
fn a_service_binding_renders_as_wranglers_services() {
    let (_root, found) = fixture_with(&format!(
        "{SPEC}  services: {{ PIGEON_CONTROL: pigeon-control }}\n"
    ));
    let v = workers::render(&found, &outputs()).expect("renders");
    assert_eq!(
        v["services"],
        json!([{ "binding": "PIGEON_CONTROL", "service": "pigeon-control" }])
    );
}

#[test]
fn the_serving_version_is_the_one_with_the_traffic() {
    let answer = json!({ "deployments": [
        { "versions": [ { "version_id": "old", "percentage": 10.0 }, { "version_id": "new", "percentage": 90.0 } ] },
        { "versions": [ { "version_id": "older", "percentage": 100.0 } ] },
    ] });
    assert_eq!(ship::serving(&answer).as_deref(), Some("new"));
    assert_eq!(ship::serving(&json!({ "deployments": [] })), None);
    assert_eq!(ship::serving(&json!(null)), None);
}

#[test]
fn health_is_the_new_versions_and_ok() {
    assert!(ship::healthy(&json!({ "ok": true, "version": "v2" }), "v2"));
    assert!(
        !ship::healthy(&json!({ "ok": true, "version": "v1" }), "v2"),
        "the old version"
    );
    assert!(!ship::healthy(
        &json!({ "ok": false, "version": "v2" }),
        "v2"
    ));
    assert!(!ship::healthy(&json!("error"), "v2"));
}

#[test]
fn a_deploy_claims_only_one_hosts_route_never_a_wildcard() {
    assert!(ship::claimable("green.platform.pigeon.dev/*"));
    assert!(ship::claimable("control.platform.pigeon.dev/*"));
    assert!(
        !ship::claimable("*.pigeon.dev/*"),
        "the front's zone wildcard"
    );
    assert!(!ship::claimable("*pigeon.dev/*"));
    assert!(!ship::claimable("/*"));
}

#[test]
fn a_probe_says_whether_the_front_forwarded_the_answer() {
    assert_eq!(agent::ship::reached(None), "direct");
    assert_eq!(agent::ship::reached(Some("")), "direct");
    assert_eq!(
        agent::ship::reached(Some("pigeon-control")),
        "via front to pigeon-control"
    );
    assert_eq!(agent::ship::FORWARDED, "x-pigeon-front");
}

#[test]
fn a_colour_or_forwarder_is_checked_on_its_own_host() {
    let (_root, found) = fixture();
    assert_eq!(ship::own_host(&found), None, "the deploy's own host");
    let mut colour = found.clone();
    colour.spec.color = true;
    assert_eq!(
        ship::own_host(&colour).as_deref(),
        Some("control.example.workers.dev")
    );
    let mut forwarder = found;
    forwarder.spec.forwards = Some("front.example.com".into());
    assert_eq!(
        ship::own_host(&forwarder).as_deref(),
        Some("front.example.com")
    );
}

#[test]
fn ids_names_and_images_are_scrubbed() {
    let images = Images {
        source: "abc".into(),
        images: BTreeMap::from([("pigeon-control/Runner".to_string(), IMAGE.to_string())]),
    };
    let s = ship::secrets_of(&outputs(), &images);
    for want in [DB, "pigeon-ci-x1", "pigeon-state-x2", IMAGE] {
        assert!(s.iter().any(|x| x == want), "{want}");
    }
    let wire = serde_json::to_string(&images).expect("json");
    assert_eq!(
        serde_json::from_str::<Images>(&wire).expect("parses"),
        images
    );
}

#[test]
fn image_inputs_block_only_on_the_image_tools() {
    let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let blocks = agent::checks::scope::image_inputs(&files(&[
        "Dockerfile",
        "src/pigeon-agent/src/jobs.rs",
        "pigeon-start",
        "Cargo.lock",
        "mise.toml",
        ".claude/decisions/x.md",
    ]));
    assert_eq!(blocks, files(&["Dockerfile", "pigeon-start", "mise.toml"]));
}

#[test]
fn a_deploy_builds_pigeon_only_when_the_workspace_changed() {
    let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    assert!(agent::checks::scope::builds(None));
    assert!(agent::checks::scope::builds(Some(&files(&[
        "src/pigeon-agent/src/jobs.rs"
    ]))));
    assert!(agent::checks::scope::builds(Some(&files(&["Cargo.lock"]))));
    assert!(!agent::checks::scope::builds(Some(&files(&[
        ".claude/decisions/x.md",
        "infra/pigeon/sites/docs/config.yaml"
    ]))));
    assert!(agent::checks::scope::builds_judge(Some(&files(&[
        "src/pigeon-eviction/src/lib.rs"
    ]))));
    assert!(!agent::checks::scope::builds_judge(Some(&files(&[
        "src/pigeon-agent/src/jobs.rs"
    ]))));
}

#[test]
fn a_deploy_is_whole_for_a_new_worker_its_own_files_or_new_images() {
    let dir = "infra/pigeon/workers/control";
    let code = vec!["src/pigeon-agent/src/lib.rs".to_string()];
    let spec = vec![format!("{dir}/worker.yaml")];
    assert_eq!(
        ship::whole_deploy(dir, Some(&code), false, Some("blue")),
        None
    );
    assert!(ship::whole_deploy(dir, Some(&code), false, None).is_some());
    assert!(ship::whole_deploy(dir, Some(&code), true, Some("blue")).is_some());
    assert!(ship::whole_deploy(dir, None, false, Some("blue")).is_some());
    let why = ship::whole_deploy(dir, Some(&spec), false, Some("blue")).unwrap_or_default();
    assert!(why.contains("worker.yaml"), "{why}");
    let sibling = vec!["infra/pigeon/workers/controller/worker.yaml".to_string()];
    assert_eq!(
        ship::whole_deploy(dir, Some(&sibling), false, Some("blue")),
        None
    );
}

#[test]
fn the_uploaded_version_is_read_from_wranglers_output() {
    let out = "Total Upload: 812.40 KiB\nWorker Startup Time: 21 ms\nUploaded pigeon-control (3.1 sec)\nWorker Version ID: 6783e1f1-7188-41f5-94dc-b45758d696f9\n";
    assert_eq!(
        ship::uploaded_version(out).as_deref(),
        Some("6783e1f1-7188-41f5-94dc-b45758d696f9")
    );
    assert_eq!(ship::uploaded_version("Uploaded pigeon-control\n"), None);
}

#[test]
fn a_smoke_passes_only_as_green_with_every_probe() {
    let green = "g";
    let pass = json!({ "ok": true, "version": "g", "probes": { "d1": true, "tools": true } });
    assert!(ship::smoke_passed(&pass, green));
    assert!(!ship::smoke_passed(&pass, "b"));
    let probe = json!({ "ok": true, "version": "g", "probes": { "d1": true, "tools": false } });
    assert!(!ship::smoke_passed(&probe, green));
    let none = json!({ "ok": true, "version": "g", "probes": {} });
    assert!(!ship::smoke_passed(&none, green));
}

#[test]
fn a_deployment_names_its_versions_and_how_objects_update() {
    let d = ship::deployment(&[("blue", 100), ("green", 0)], false);
    assert_eq!(
        d["versions"][1],
        json!({ "version_id": "green", "percentage": 0 })
    );
    assert_eq!(
        d["code_update_strategy"],
        json!({ "mode": "deferred", "max_delay": ship::CODE_UPDATE_DELAY })
    );
    let back = ship::deployment(&[("blue", 100)], true);
    assert_eq!(back["code_update_strategy"], json!({ "mode": "immediate" }));
    assert!(ship::HEALTH_WAIT.as_secs() > u64::from(ship::CODE_UPDATE_DELAY));
}

#[test]
fn a_var_holding_an_output_reference_renders_as_its_value() {
    let (_root, found) = fixture();
    let images = BTreeMap::from([("pigeon-control/Runner".to_string(), IMAGE.to_string())]);
    let v = workers::render_with(&found, &outputs(), Some(&images)).expect("renders");
    assert_eq!(v["vars"]["STATE_BUCKET"], json!("pigeon-state-x2"));
    assert_eq!(
        v["vars"]["PUBLIC_HOST"],
        json!("control.example.workers.dev")
    );
    let snap = ship::snapshot(&[found.spec], &outputs());
    assert!(snap["infra/pigeon"].contains_key("bucket_names"));
}

#[test]
fn a_var_whose_output_is_missing_renders_as_written() {
    let (_root, found) = fixture();
    let images = BTreeMap::from([("pigeon-control/Runner".to_string(), IMAGE.to_string())]);
    let mut outs = outputs();
    outs.get_mut("infra/pigeon")
        .expect("unit")
        .insert("bucket_names".into(), json!(["pigeon-ci-x1"]));
    let v = workers::render_with(&found, &outs, Some(&images)).expect("still renders");
    assert_eq!(
        v["vars"]["STATE_BUCKET"],
        json!("$output.infra/pigeon.bucket_names[pigeon-state]")
    );
}

#[test]
fn builds_go_to_the_platform_workers_bucket_never_a_colours() {
    let root = TempDir::new().expect("tempdir");
    let p = root.path();
    fs::create_dir_all(p.join("crates/agent")).expect("mkdir");
    fs::write(p.join("crates/agent/Cargo.toml"), "[package]\n").expect("write");
    let spec = |name: &str, color: bool, bucket: &str| {
        format!(
            "description: A colour.\nname: {name}\ncolor: {color}\nentry: {{ crate: crates/agent, features: [worker] }}\ncompatibility_date: \"2026-10-01\"\nbindings:\n  r2: {{ CI_LOGS: \"$output.infra/pigeon.bucket_names[{bucket}]\" }}\n"
        )
    };
    // `color/` sorts before `control/`, so a search by order alone would pick green's bucket.
    for (dir, text) in [
        ("color", spec("pigeon-green", true, "green-ci")),
        ("control", spec("pigeon-control", false, "pigeon-ci")),
    ] {
        fs::create_dir_all(p.join(format!("infra/pigeon/workers/{dir}"))).expect("mkdir");
        fs::write(
            p.join(format!("infra/pigeon/workers/{dir}/worker.yaml")),
            text,
        )
        .expect("write");
    }
    assert_eq!(
        agent::ship::builds_reference(p, None).as_deref(),
        Ok("$output.infra/pigeon.bucket_names[pigeon-ci]")
    );
}
