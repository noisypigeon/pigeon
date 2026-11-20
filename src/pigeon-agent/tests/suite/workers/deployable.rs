//! `pigeon workers deployable`: stub outputs, unresolved bindings and main's build; see .claude/decisions/2026-11-19-development-workers-deployable-before-merge.md.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use agent::workers::deployable::{
    BUILD_FILE, BUILD_VAR, dry_config, head_pass, main_build, main_pass, stub_outputs,
};
use agent::workers::{self, Found};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const STACK_SPEC: &str = r#"description: The platform's control plane.
name: $deployment.worker
entry: { crate: crates/agent, features: [worker] }
compatibility_date: "2026-10-01"
deployment: pigeon/platform
bindings:
  d1: { DB: { name: $deployment.database_name, id: $deployment.database_id } }
  r2: { CI_LOGS: "$deployment.bucket_names[pigeon-ci]" }
  queues: { consumers: [ { queue: $deployment.queue_name, max_batch_size: 10 } ] }
  vars: { PUBLIC_HOST: $deployment.hostname, CHANGE_FLOOR: $deployment.change_floor }
"#;

fn fixture(spec: &str) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let p = root.path();
    for d in ["infra/pigeon/workers/control", "crates/agent/migrations"] {
        fs::create_dir_all(p.join(d)).expect("mkdir");
    }
    fs::write(p.join("crates/agent/Cargo.toml"), "[package]\n").expect("write");
    fs::write(p.join("Dockerfile"), "FROM scratch AS agent\n").expect("write");
    fs::write(p.join("infra/pigeon/workers/control/worker.yaml"), spec).expect("write");
    root
}

fn stacks(root: &Path) {
    let p = root;
    fs::create_dir_all(p.join("catalog/pigeon/kinds/platform")).expect("mkdir");
    fs::write(
        p.join("catalog/pigeon/kinds/platform/schema.json"),
        r#"{"type":"object","x-pigeon":{"version":"0.1.0","local":true,"outputs":{"database_id":"$module.d1.id","bucket_names":"$module.buckets[*].name"}}}"#,
    )
    .expect("write");
    fs::write(
        p.join("config.yaml"),
        "schema: v1\nname: pigeon\ndescription: d\nrepository: https://example.com/r\nmoves: []\nunits:\n  platform/blue:\n    kind: pigeon/platform\n    worker: pigeon-control\n    database_name: pigeon\n    hostname: blue.example.com\n    change_floor: 12\n  platform/green:\n    kind: pigeon/platform\n    worker: pigeon-green\n    database_name: pigeon-green\n    hostname: green.example.com\n    change_floor: 1\n",
    )
    .expect("write");
}

fn found(root: &Path) -> Found {
    workers::discover(root)
        .expect("discovers")
        .into_iter()
        .next()
        .expect("one")
}

fn clean_spec() -> String {
    STACK_SPEC
        .lines()
        .filter(|l| !l.contains("queues:"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn messages(root: &Path) -> String {
    head_pass(root, None)
        .expect_err("fails")
        .into_iter()
        .map(|p| p.message)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_clean_stack_spec_is_deployable_on_every_stack() {
    let root = fixture(&clean_spec());
    stacks(root.path());
    let pairs = head_pass(root.path(), None).expect("deployable");
    assert_eq!(pairs, 2);
}

#[test]
fn a_raw_binding_left_by_the_binder_fails() {
    let root = fixture(STACK_SPEC);
    stacks(root.path());
    let m = messages(root.path());
    assert!(m.contains("/queues/consumers/0/queue"), "{m}");
    assert!(m.contains("$output.platform/blue.queue_name"), "{m}");
}

#[test]
fn a_typo_in_a_deployment_key_fails() {
    let root = fixture(&clean_spec().replace("$deployment.database_id", "$deployment.databse_id"));
    stacks(root.path());
    let m = messages(root.path());
    assert!(m.contains("declares no output `databse_id`"), "{m}");
}

#[test]
fn stubs_take_the_shape_each_reference_asks() {
    let root = fixture(&clean_spec());
    stacks(root.path());
    let blue = workers::bind(root.path(), &found(root.path()), "platform/blue").expect("binds");
    let (out, problems) = stub_outputs(root.path(), &blue);
    assert!(problems.is_empty(), "{problems:?}");
    let unit = &out["platform/blue"];
    assert_eq!(unit["database_id"], json!("0".repeat(32)));
    assert_eq!(unit["bucket_names"], json!(["pigeon-ci"]));
}

#[test]
fn the_dry_run_config_has_a_stub_entry_and_no_build() {
    let root = fixture(&clean_spec().replace(
        "bindings:",
        "durable_objects:\n  classes: [Instance]\n  migrations: [ { tag: v1, new_sqlite_classes: [Instance] } ]\nbindings:",
    ));
    stacks(root.path());
    let blue = workers::bind(root.path(), &found(root.path()), "platform/blue").expect("binds");
    let (out, _) = stub_outputs(root.path(), &blue);
    let rendered = workers::render(&blue, &out).expect("renders");
    let (config, module) = dry_config(&rendered, &blue.spec);
    assert!(config.get("build").is_none());
    assert_eq!(config["main"], "stub.mjs");
    assert!(module.contains("export class Instance"), "{module}");
}

#[test]
fn main_build_is_taken_only_when_its_sum_matches() {
    let tmp = TempDir::new().expect("tempdir");
    let bytes = b"not a real build";
    fs::write(tmp.path().join(BUILD_FILE), bytes).expect("write");
    let sum = Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    let dir = tmp.path().display().to_string();
    let env = |sum: Option<String>| {
        let m: BTreeMap<String, String> = [
            (BUILD_VAR.to_string(), sum),
            ("TMPDIR".into(), Some(dir.clone())),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect();
        move |k: &str| m.get(k).cloned()
    };
    assert_eq!(
        main_build(env(Some(sum)), None).expect("ok"),
        Some(tmp.path().join(BUILD_FILE))
    );
    let err = main_build(env(Some("0".repeat(64))), None).expect_err("wrong sum");
    assert!(err.contains(BUILD_VAR), "{err}");
    assert_eq!(main_build(env(None), None).expect("ok"), None);
}

fn fake_pigeon(dir: &Path, has_subcommand: bool, log: &Path) -> PathBuf {
    let path = dir.join("pigeon");
    let script = format!(
        r#"#!/bin/sh
echo "$* wrangler=$PIGEON_WRANGLER build=${{PIGEON_BUILD_SHA256:-unset}}" >> "{log}"
case "$*" in
  "workers deployable --help") {help} ;;
  "workers deployable"*) exit 0 ;;
  "workers validate"*) echo 'unknown field `host`'; exit 1 ;;
esac
"#,
        log = log.display(),
        help = if has_subcommand { "exit 0" } else { "exit 1" }
    );
    fs::write(&path, script).expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[test]
fn an_old_main_build_that_rejects_a_field_fails_the_main_pass() {
    let tmp = TempDir::new().expect("tempdir");
    let log = tmp.path().join("log");
    let main = fake_pigeon(tmp.path(), false, &log);
    let lines = main_pass(&main, tmp.path()).expect_err("fails");
    assert!(
        lines[0].contains("a spec field ships one change after its parser"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("unknown field `host`")),
        "{lines:?}"
    );
}

#[test]
fn a_main_build_with_the_subcommand_runs_it_on_the_head() {
    let tmp = TempDir::new().expect("tempdir");
    let log = tmp.path().join("log");
    let main = fake_pigeon(tmp.path(), true, &log);
    assert_eq!(main_pass(&main, tmp.path()), Ok("deployable"));
    let logged = fs::read_to_string(&log).expect("log");
    assert!(logged.contains("workers deployable --root"), "{logged}");
    assert!(
        logged.contains("--pass head wrangler=/nonexistent/wrangler build=unset"),
        "{logged}"
    );
}

#[test]
fn an_underscored_output_stubs_as_a_valid_name() {
    let root = fixture(&clean_spec().replace(
        "\"$deployment.bucket_names[pigeon-ci]\"",
        "$deployment.ci_bucket",
    ));
    stacks(root.path());
    fs::write(
        root.path().join("catalog/pigeon/kinds/platform/schema.json"),
        r#"{"type":"object","x-pigeon":{"version":"0.1.0","local":true,"outputs":{"database_id":"$module.d1.id","ci_bucket":"$module.bucket.name"}}}"#,
    )
    .expect("write");
    let blue = workers::bind(root.path(), &found(root.path()), "platform/blue").expect("binds");
    let (out, problems) = stub_outputs(root.path(), &blue);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(out["platform/blue"]["ci_bucket"], json!("stub-ci-bucket"));
}

#[test]
fn this_repositorys_specs_are_deployable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = head_pass(&root, None);
    assert!(result.is_ok(), "{:?}", result.err());
}
