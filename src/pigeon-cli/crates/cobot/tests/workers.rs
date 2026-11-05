//! Worker specs, rendering and deploys with fake wrangler and docker; see
//! `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use cobot::workers::{self, Found, Tools};
use serde_json::{Map, Value, json};
use tempfile::TempDir;

const ACCOUNT: &str = "0123456789abcdef0123456789ab305a";
const DB: &str = "fedcba9876543210fedcba9876543210";

const SPEC: &str = r#"description: The platform's control plane.
name: pigeon-control
entry: { crate: crates/backplane, features: [worker] }
compatibility_date: "2026-10-01"
routes: [ { custom_domain: pigeon.dev } ]
crons: ["17 3 * * *"]
durable_objects:
  classes: [Instance, Runner]
  migrations: [ { tag: v1, new_sqlite_classes: [Instance, Runner] } ]
containers:
  - { class: Runner, image: { dockerfile: Dockerfile, target: backplane, args: { FEATURES: backplane } }, instance_type: standard-1, max_instances: 4 }
bindings:
  d1: { DB: { name: pigeon, id: $output.infra/pigeon.database_id, migrations: crates/backplane/migrations } }
  r2: { STATE: "$output.infra/pigeon.bucket_names[0]" }
  queues: { producers: { EVENTS: ci-events }, consumers: [ { queue: ci-events, max_batch_size: 10 } ] }
  vars: { TEAM: pigeon }
secrets: [GRANT_KEY]
"#;

fn fixture(spec: &str) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let p = root.path();
    for d in [
        "infra/pigeon/workers/control",
        "crates/backplane/migrations",
    ] {
        fs::create_dir_all(p.join(d)).expect("mkdir");
    }
    fs::write(p.join("crates/backplane/Cargo.toml"), "[package]\n").expect("write");
    fs::write(p.join("Dockerfile"), "FROM scratch AS backplane\n").expect("write");
    fs::write(p.join("infra/pigeon/workers/control/worker.yaml"), spec).expect("write");
    root
}

fn found(root: &Path) -> Found {
    let all = workers::discover(root).expect("discovers");
    assert_eq!(all.len(), 1);
    all.into_iter().next().expect("one")
}

fn outputs() -> BTreeMap<String, Map<String, Value>> {
    let mut m = Map::new();
    m.insert("database_id".into(), json!(DB));
    m.insert(
        "bucket_names".into(),
        json!(["pigeon-state", "pigeon-plans"]),
    );
    BTreeMap::from([("infra/pigeon".to_string(), m)])
}

#[test]
fn a_spec_validates_and_its_unit_is_found() {
    let root = fixture(SPEC);
    let f = found(root.path());
    assert_eq!(f.unit, "infra/pigeon");
    assert_eq!(f.dir, "infra/pigeon/workers/control");
    let problems = workers::validate(root.path(), &f);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        workers::referenced_units(&f.spec)
            .into_iter()
            .collect::<Vec<_>>(),
        ["infra/pigeon"]
    );
}

#[test]
fn bad_specs_are_refused_with_every_problem() {
    let bad = SPEC
        .replace("name: pigeon-control", "name: Pigeon_Control")
        .replace("\n  classes: [Instance, Runner]", "\n  classes: [Instance]")
        .replace("standard-1", "huge")
        .replace("$output.infra/pigeon.database_id", "abc")
        .replace("TEAM: pigeon", &format!("team: {ACCOUNT}"));
    let root = fixture(&bad);
    let got: Vec<String> = workers::validate(root.path(), &found(root.path()))
        .into_iter()
        .map(|p| p.message)
        .collect();
    for want in [
        "`name` `Pigeon_Control`",
        "migration `v1` names unknown class `Runner`",
        "container `Runner` must also be a Durable Object class",
        "instance type `huge`",
        "`abc` must be a `$output.<unit>.<name>` reference",
        "binding `team` must be SCREAMING_SNAKE_CASE",
        "var `team` looks like an ID",
    ] {
        assert!(got.iter().any(|m| m.contains(want)), "{want}: {got:?}");
    }
    let unknown = fixture(&format!("{SPEC}zone: x\n"));
    assert!(matches!(
        workers::discover(unknown.path()),
        Err(cobot::Error::Invalid(p)) if p[0].message.contains("zone")
    ));
}

#[test]
fn references_resolve_and_a_missing_output_is_named() {
    assert_eq!(
        workers::reference("$output.infra/pigeon.bucket_names[1]")
            .map(|r| (r.unit, r.output, r.index)),
        Some((
            "infra/pigeon".into(),
            "bucket_names".into(),
            Some(workers::Item::At(1))
        ))
    );
    assert_eq!(workers::reference("$output.pigeon.x"), None);
    let mut suffixed = outputs();
    suffixed.get_mut("infra/pigeon").expect("unit").insert(
        "bucket_names".into(),
        json!(["pigeon-ci-x1", "pigeon-ci-extra-y2", "pigeon-state-z3"]),
    );
    assert_eq!(
        workers::resolve("$output.infra/pigeon.bucket_names[pigeon-ci]", &suffixed).as_deref(),
        Ok("pigeon-ci-x1"),
        "a name picks its suffixed item, not a longer name's"
    );
    assert!(
        workers::resolve("$output.infra/pigeon.bucket_names[pigeon-plans]", &suffixed).is_err()
    );
    assert_eq!(workers::reference("$output.infra/pigeon.x[Bad]"), None);
    assert_eq!(
        workers::resolve("$output.infra/pigeon.bucket_names[1]", &outputs()).as_deref(),
        Ok("pigeon-plans")
    );
    let err = workers::resolve("$output.infra/pigeon.store_id", &outputs()).unwrap_err();
    assert!(err.contains("has no output `store_id` yet"), "{err}");
}

#[test]
fn render_is_stable_and_resolves_every_binding() {
    let root = fixture(SPEC);
    let f = found(root.path());
    let w = workers::render(&f, &outputs()).expect("renders");
    assert_eq!(
        workers::text(&w),
        workers::text(&workers::render(&f, &outputs()).expect("again"))
    );
    assert_eq!(w["main"], "../../../crates/backplane/build/worker/shim.mjs");
    assert_eq!(
        w["build"]["command"],
        "worker-build --release --no-default-features --features worker"
    );
    assert_eq!(w["d1_databases"][0]["database_id"], DB);
    assert_eq!(
        w["d1_databases"][0]["migrations_dir"],
        "../../../crates/backplane/migrations"
    );
    assert_eq!(w["r2_buckets"][0]["bucket_name"], "pigeon-state");
    assert_eq!(
        w["durable_objects"]["bindings"][1],
        json!({ "name": "RUNNER", "class_name": "Runner" })
    );
    assert_eq!(w["containers"][0]["image"], "./Runner.Dockerfile");
    assert_eq!(
        w["routes"][0],
        json!({ "pattern": "pigeon.dev", "custom_domain": true })
    );
    assert_eq!(w["triggers"]["crons"][0], "17 3 * * *");
    assert_eq!(w["queues"]["consumers"][0]["max_batch_size"], 10);
    assert_eq!(w["workers_dev"], false);
    assert!(workers::render(&f, &BTreeMap::new()).is_err());
    assert_eq!(workers::binding_of("ChangeRun"), "CHANGE_RUN");
}

#[test]
fn a_plan_expires_and_a_bad_id_is_refused() {
    let state = TempDir::new().expect("tempdir");
    let rendered = BTreeMap::from([("pigeon-control".to_string(), json!({ "name": "x" }))]);
    let stored = workers::store(state.path(), &rendered, 1000).expect("stores");
    let back = workers::load(state.path(), &stored.id, 1001).expect("loads");
    assert_eq!(
        back.workers["pigeon-control"],
        workers::digest(&json!({ "name": "x" }))
    );
    assert!(matches!(
        workers::load(state.path(), &stored.id, stored.expires),
        Err(cobot::Error::Plan(m)) if m.contains("expired")
    ));
    assert!(matches!(
        workers::load(state.path(), "../x", 1001),
        Err(cobot::Error::Plan(_))
    ));
    assert!(matches!(
        workers::load(state.path(), &"0".repeat(32), 1001),
        Err(cobot::Error::Plan(m)) if m.contains("no stored")
    ));
}

#[test]
fn permissions_follow_the_bindings() {
    let root = fixture(SPEC);
    let got = workers::permissions(&[found(root.path()).spec]);
    for want in [
        "Workers Scripts Write (Workers Admin)",
        "Workers Containers Read and Write",
        "D1 Read and Write",
        "Workers R2 Storage Read and Write",
        "Queues Read and Write",
        "SSL and Certificates Read and Write",
    ] {
        assert!(got.contains(want), "{want}: {got:?}");
    }
    assert!(!got.contains("Secrets Store Read and Write"));
}

#[test]
fn scrubbing_hides_ids_and_secrets() {
    let shown = workers::scrub(
        &format!("account {ACCOUNT} token sekrit-token-value ok"),
        &["sekrit-token-value"],
    );
    assert_eq!(shown, "account …305a token … ok");
}

/// A fake tool that logs its arguments, one call per line, and answers listings.
fn fake(dir: &Path, name: &str, log: &Path) -> PathBuf {
    let path = dir.join(name);
    let script = format!(
        r#"#!/bin/sh
echo "{name} $* | $CLOUDFLARE_ACCOUNT_ID" >> "{log}"
case "$*" in
  "secret list"*) echo '[{{"name":"OTHER"}}]' ;;
  "containers list"*) echo '[{{"id":"{ACCOUNT}","name":"pigeon-control-runner"}},{{"id":"x","name":"other"}}]' ;;
  "deploy"*) echo "deployed to account {ACCOUNT}" ;;
  *fail*) echo "failed in {ACCOUNT}" >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    fs::write(&path, script).expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn tools(dir: &Path, log: &Path) -> Tools {
    Tools {
        wrangler: fake(dir, "wrangler", log),
        docker: fake(dir, "docker", log),
        env: vec![("CLOUDFLARE_ACCOUNT_ID".into(), ACCOUNT.into())],
        secrets: Vec::new(),
    }
}

#[test]
fn deploy_migrates_builds_then_deploys_and_shows_no_id() {
    let root = fixture(SPEC);
    let bin = TempDir::new().expect("tempdir");
    let log = bin.path().join("log");
    let t = tools(bin.path(), &log);
    let f = found(root.path());
    let plan = "0123456789abcdef0123456789abcdef";
    let rendered = workers::render(&f, &outputs()).expect("renders");
    let dir = workers::write(root.path(), &f, &rendered, plan).expect("writes");
    assert_eq!(
        fs::read_to_string(dir.join("Runner.Dockerfile"))
            .expect("read")
            .lines()
            .last(),
        Some(format!("FROM pigeon-local/pigeon-control-runner:{plan}").as_str())
    );
    let done = workers::deploy(root.path(), &f, &dir, plan, &t).expect("deploys");
    assert_eq!(done.len(), 3, "{done:?}");
    let calls: Vec<String> = fs::read_to_string(&log)
        .expect("log")
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        calls[0].starts_with("wrangler d1 migrations apply pigeon --remote"),
        "{calls:?}"
    );
    assert!(
        calls[1]
            .starts_with("docker build --target backplane --build-arg FEATURES=backplane -t pigeon-local/pigeon-control-runner:"),
        "{calls:?}"
    );
    assert!(
        calls[2].starts_with("wrangler deploy -c wrangler.jsonc"),
        "{calls:?}"
    );
    assert!(
        calls.iter().all(|c| c.ends_with(ACCOUNT)),
        "the account reaches wrangler"
    );
    assert_eq!(
        workers::missing_secrets(&f, &t, &dir).expect("lists"),
        ["GRANT_KEY"]
    );
    let gone = workers::teardown(&f, &t, &dir).expect("tears down");
    assert_eq!(
        gone,
        [
            "pigeon-control deleted",
            "container application pigeon-control-runner deleted"
        ]
    );
}

#[test]
fn a_failing_step_stops_and_its_output_is_scrubbed() {
    let root = fixture(
        SPEC.replace("name: pigeon\n", "name: fail\n")
            .replace("{ name: pigeon,", "{ name: fail,")
            .as_str(),
    );
    let bin = TempDir::new().expect("tempdir");
    let log = bin.path().join("log");
    let t = tools(bin.path(), &log);
    let f = found(root.path());
    let dir = workers::write(
        root.path(),
        &f,
        &workers::render(&f, &outputs()).expect("renders"),
        "p",
    )
    .expect("writes");
    let err = workers::deploy(root.path(), &f, &dir, "p", &t)
        .unwrap_err()
        .to_string();
    assert!(err.contains("failed in …"), "{err}");
    assert!(!err.contains(ACCOUNT), "{err}");
    assert_eq!(
        fs::read_to_string(&log).expect("log").lines().count(),
        1,
        "nothing after the failure"
    );
}

#[test]
fn a_secret_is_put_through_stdin_and_never_shown() {
    let root = fixture(SPEC);
    let bin = TempDir::new().expect("tempdir");
    let log = bin.path().join("log");
    let t = tools(bin.path(), &log);
    let f = found(root.path());
    workers::put_secret(&f, "GRANT_KEY", "the-secret-value", &t, root.path()).expect("puts");
    let logged = fs::read_to_string(&log).expect("log");
    assert!(logged.contains("secret put GRANT_KEY --name pigeon-control"));
    assert!(!logged.contains("the-secret-value"));
    assert!(matches!(
        workers::put_secret(&f, "NOPE", "v", &t, root.path()),
        Err(cobot::Error::Invalid(_))
    ));
}

#[test]
fn this_repositorys_specs_validate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    for f in workers::discover(&root).expect("discovers") {
        let problems = workers::validate(&root, &f);
        assert!(problems.is_empty(), "{}: {problems:?}", f.dir);
    }
}
