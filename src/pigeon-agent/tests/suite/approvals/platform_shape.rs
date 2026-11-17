//! The `pigeon/platform` kind's `shape`: `full` needs the whole stack, `app` only a Worker, its
//! instance and one bucket, with no domain; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`, its 2026-11-15 amendment.
#![cfg(feature = "engines")]

use serde_json::{Value, json};

fn valid(unit: &Value) -> bool {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../../../catalog/pigeon/kinds/platform/schema.json"
    ))
    .expect("schema parses");
    jsonschema::validator_for(&schema)
        .expect("schema compiles")
        .is_valid(unit)
}

fn app() -> Value {
    json!({ "shape": "app", "worker": "learner", "instance": "learner",
            "domains": [], "buckets": [{ "name": "learner-0a1b2c3d" }] })
}

#[test]
fn an_app_shape_needs_no_domain_database_queue_or_access() {
    assert!(valid(&app()));
}

#[test]
fn an_app_shape_has_no_domain_and_one_bucket() {
    let mut with_domain = app();
    with_domain["domains"] = json!(["example.com"]);
    assert!(!valid(&with_domain));
    let mut no_bucket = app();
    no_bucket["buckets"] = json!([]);
    assert!(!valid(&no_bucket));
    let mut no_instance = app();
    no_instance.as_object_mut().unwrap().remove("instance");
    assert!(!valid(&no_instance));
}

#[test]
fn a_full_shape_still_needs_the_whole_stack() {
    let mut full = app();
    full["shape"] = json!("full");
    full["domains"] = json!(["example.com"]);
    assert!(!valid(&full));
    let mut unset = full.clone();
    unset.as_object_mut().unwrap().remove("shape");
    assert!(!valid(&unset));
}

#[test]
fn the_app_worker_template_is_a_worker_spec_with_no_container_or_domain() {
    let spec: agent::workers::Spec = serde_saphyr::from_str(include_str!(
        "../../../../../catalog/pigeon/kinds/platform/app-worker.yaml"
    ))
    .expect("template parses");
    assert!(spec.workers_dev);
    assert!(spec.routes.is_empty() && spec.containers.is_empty() && spec.crons.is_empty());
    assert_eq!(spec.durable_objects.classes, ["Instance"]);
    assert_eq!(spec.bindings.vars["SHAPE"], "app");
    assert_eq!(spec.secrets, ["STATE_KEY", "HANDOVER", "BASE_URL"]);
}
