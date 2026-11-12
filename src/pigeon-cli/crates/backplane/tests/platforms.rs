//! Platform runs: offers, grants, the sealed token's lifetime; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

use backplane::platforms::{Kept, LIFETIME, Runs, Stage, finish_event, merged_outputs};
use serde_json::json;

#[test]
fn a_run_is_planned_then_applied_each_on_its_own_grant() {
    let mut runs = Runs::default();
    let r = stepped!(runs.propose("platform/blue", "g1", 100)).expect("proposes");
    assert_eq!((r.id, r.stage), (1, Stage::Offered));
    assert!(runs.peek(1, "wrong", Stage::Offered, 101).is_err());
    let r = stepped!(runs.grant_plan(1, "g1", "SEALED", "owner@example.com", 101)).expect("grants");
    assert_eq!(r.stage, Stage::Planning);
    assert_eq!(r.argv(), ["plan", "platform/blue"]);
    assert_eq!(r.job_id().as_deref(), Some("platform-1-plan"));
    // A grant is used once.
    assert!(stepped!(runs.grant_plan(1, "g1", "SEALED", "x", 102)).is_err());
    let r = stepped!(runs.planned(
        1,
        Ok(("p1".into(), vec![json!({"address": "a"})])),
        "g2",
        200,
    ))
    .expect("planned");
    assert_eq!((r.stage, r.plan.as_deref()), (Stage::Planned, Some("p1")));
    assert_eq!(r.sealed.as_deref(), Some("SEALED"));
    let r = stepped!(runs.grant_apply(1, "g2", "owner@example.com", 201)).expect("grants");
    assert_eq!(r.argv(), ["apply", "platform/blue", "p1"]);
    let r = stepped!(runs.applied(1, Ok(Vec::new()))).expect("applied");
    assert_eq!(r.stage, Stage::Applied);
    assert_eq!(r.sealed, None, "the token is erased once applied");
    assert_eq!(r.granted_by.len(), 2);
}

#[test]
fn a_failed_or_refused_run_erases_its_token() {
    let mut runs = Runs::default();
    stepped!(runs.propose("platform/blue", "g1", 0)).expect("proposes");
    stepped!(runs.grant_plan(1, "g1", "SEALED", "o", 1)).expect("grants");
    let r = stepped!(runs.planned(1, Err("the plan destroys x".into()), "g2", 2)).expect("records");
    assert_eq!((r.stage, r.sealed.clone()), (Stage::Failed, None));
    stepped!(runs.propose("platform/blue", "g3", 3)).expect("a new run once the last finished");
    stepped!(runs.grant_plan(2, "g3", "SEALED", "o", 4)).expect("grants");
    stepped!(runs.planned(2, Ok(("p".into(), Vec::new())), "g4", 5)).expect("planned");
    let r = stepped!(runs.refuse(2, "o")).expect("refuses");
    assert_eq!((r.stage, r.sealed), (Stage::Failed, None));
}

#[test]
fn one_open_run_per_unit_and_only_platform_units() {
    let mut runs = Runs::default();
    stepped!(runs.propose("platform/blue", "g", 0)).expect("proposes");
    assert!(stepped!(runs.propose("platform/blue", "h", 1)).is_err());
    assert!(stepped!(runs.propose("platform/green", "h", 1)).is_ok());
    assert!(stepped!(runs.propose("infra/pigeon", "h", 1)).is_err());
}

#[test]
fn a_waiting_run_expires_with_its_token_but_a_running_job_is_left_alone() {
    let mut runs = Runs::default();
    stepped!(runs.propose("platform/blue", "g1", 0)).expect("proposes");
    stepped!(runs.grant_plan(1, "g1", "SEALED", "o", 1)).expect("grants");
    stepped!(runs.planned(1, Ok(("p".into(), Vec::new())), "g2", 2)).expect("planned");
    stepped!(runs.propose("platform/green", "g3", 0)).expect("proposes");
    stepped!(runs.grant_plan(2, "g3", "SEALED", "o", 1)).expect("grants");
    assert_eq!(kept!(runs.expire(LIFETIME - 1)), Vec::<u32>::new());
    assert_eq!(kept!(runs.expire(LIFETIME)), vec![1]);
    let r = runs.get(1).expect("run");
    assert_eq!((r.stage, r.sealed.clone()), (Stage::Expired, None));
    assert_eq!(runs.get(2).expect("run").stage, Stage::Planning);
    assert!(stepped!(runs.grant_apply(1, "g2", "o", LIFETIME)).is_err());
}

#[test]
fn a_shown_run_holds_nothing_secret() {
    let mut runs = Runs::default();
    stepped!(runs.propose("platform/blue", "g1", 0)).expect("proposes");
    let r = stepped!(runs.grant_plan(1, "g1", "SEALED", "o", 1))
        .expect("grants")
        .shown();
    let text = serde_json::to_string(&r).expect("serializes");
    assert!(!text.contains("SEALED"), "{text}");
}

#[test]
fn a_finished_run_says_what_happened() {
    let mut runs = Runs::default();
    stepped!(runs.propose("platform/blue", "g1", 0)).expect("proposes");
    assert_eq!(finish_event(runs.get(1).expect("run")), None);
    stepped!(runs.grant_plan(1, "g1", "SEALED", "o", 1)).expect("grants");
    let r = stepped!(runs.planned(1, Ok(("p".into(), vec![json!({}), json!({})])), "g2", 2))
        .expect("planned");
    let (kind, said) = finish_event(&r).expect("an event");
    assert_eq!(kind, "platform.planned");
    assert!(
        said.contains("run 1 of platform/blue planned: 2 change(s)"),
        "{said}"
    );
    stepped!(runs.grant_apply(1, "g2", "o", 3)).expect("grants");
    let r = stepped!(runs.applied(1, Err("tofu failed".into()))).expect("records");
    let (kind, said) = finish_event(&r).expect("an event");
    assert_eq!(kind, "platform.failed");
    assert!(said.ends_with("failed: tofu failed"), "{said}");
}

#[test]
fn kept_outputs_replace_their_unit_in_the_secret() {
    let secret = r#"{"infra/pigeon":{"database_id":"old"},"platform/blue":{"queue_name":"old"}}"#;
    assert_eq!(
        merged_outputs(Some(secret), &Kept::new()).as_deref(),
        Some(secret)
    );
    assert_eq!(merged_outputs(None, &Kept::new()), None);
    let mut kept = Kept::new();
    let outputs = json!({ "queue_name": "pigeon-ci-events", "database_id": "d" });
    kept.insert(
        "platform/blue".into(),
        outputs.as_object().cloned().expect("object"),
    );
    let got: serde_json::Value =
        serde_json::from_str(&merged_outputs(Some(secret), &kept).expect("merged")).expect("JSON");
    assert_eq!(got["infra/pigeon"]["database_id"], "old");
    assert_eq!(got["platform/blue"]["queue_name"], "pigeon-ci-events");
    assert_eq!(got["platform/blue"]["database_id"], "d");
    let alone: serde_json::Value =
        serde_json::from_str(&merged_outputs(None, &kept).expect("merged")).expect("JSON");
    assert_eq!(alone["platform/blue"]["database_id"], "d");
}
