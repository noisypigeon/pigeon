//! Platform runs: offers, grants, the sealed token's lifetime; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use backplane::platforms::{LIFETIME, Runs, Stage};
use serde_json::json;

#[test]
fn a_run_is_planned_then_applied_each_on_its_own_grant() {
    let mut runs = Runs::default();
    let r = runs.propose("platform/blue", "g1", 100).expect("proposes");
    assert_eq!((r.id, r.stage), (1, Stage::Offered));
    assert!(runs.peek(1, "wrong", Stage::Offered, 101).is_err());
    let r = runs
        .grant_plan(1, "g1", "SEALED", "owner@example.com", 101)
        .expect("grants");
    assert_eq!(r.stage, Stage::Planning);
    assert_eq!(r.argv(), ["plan", "platform/blue"]);
    assert_eq!(r.job_id().as_deref(), Some("platform-1-plan"));
    // A grant is used once.
    assert!(runs.grant_plan(1, "g1", "SEALED", "x", 102).is_err());
    let r = runs
        .planned(
            1,
            Ok(("p1".into(), vec![json!({"address": "a"})])),
            "g2",
            200,
        )
        .expect("planned");
    assert_eq!((r.stage, r.plan.as_deref()), (Stage::Planned, Some("p1")));
    assert_eq!(r.sealed.as_deref(), Some("SEALED"));
    let r = runs
        .grant_apply(1, "g2", "owner@example.com", 201)
        .expect("grants");
    assert_eq!(r.argv(), ["apply", "platform/blue", "p1"]);
    let r = runs.applied(1, Ok(Vec::new())).expect("applied");
    assert_eq!(r.stage, Stage::Applied);
    assert_eq!(r.sealed, None, "the token is erased once applied");
    assert_eq!(r.granted_by.len(), 2);
}

#[test]
fn a_failed_or_refused_run_erases_its_token() {
    let mut runs = Runs::default();
    runs.propose("platform/blue", "g1", 0).expect("proposes");
    runs.grant_plan(1, "g1", "SEALED", "o", 1).expect("grants");
    let r = runs
        .planned(1, Err("the plan destroys x".into()), "g2", 2)
        .expect("records");
    assert_eq!((r.stage, r.sealed.clone()), (Stage::Failed, None));
    runs.propose("platform/blue", "g3", 3)
        .expect("a new run once the last finished");
    runs.grant_plan(2, "g3", "SEALED", "o", 4).expect("grants");
    runs.planned(2, Ok(("p".into(), Vec::new())), "g4", 5)
        .expect("planned");
    let r = runs.refuse(2, "o").expect("refuses");
    assert_eq!((r.stage, r.sealed), (Stage::Failed, None));
}

#[test]
fn one_open_run_per_unit_and_only_platform_units() {
    let mut runs = Runs::default();
    runs.propose("platform/blue", "g", 0).expect("proposes");
    assert!(runs.propose("platform/blue", "h", 1).is_err());
    assert!(runs.propose("platform/green", "h", 1).is_ok());
    assert!(runs.propose("infra/pigeon", "h", 1).is_err());
}

#[test]
fn a_waiting_run_expires_with_its_token_but_a_running_job_is_left_alone() {
    let mut runs = Runs::default();
    runs.propose("platform/blue", "g1", 0).expect("proposes");
    runs.grant_plan(1, "g1", "SEALED", "o", 1).expect("grants");
    runs.planned(1, Ok(("p".into(), Vec::new())), "g2", 2)
        .expect("planned");
    runs.propose("platform/green", "g3", 0).expect("proposes");
    runs.grant_plan(2, "g3", "SEALED", "o", 1).expect("grants");
    assert_eq!(runs.expire(LIFETIME - 1), Vec::<u32>::new());
    assert_eq!(runs.expire(LIFETIME), vec![1]);
    let r = runs.get(1).expect("run");
    assert_eq!((r.stage, r.sealed.clone()), (Stage::Expired, None));
    assert_eq!(runs.get(2).expect("run").stage, Stage::Planning);
    assert!(runs.grant_apply(1, "g2", "o", LIFETIME).is_err());
}

#[test]
fn a_shown_run_holds_nothing_secret() {
    let mut runs = Runs::default();
    runs.propose("platform/blue", "g1", 0).expect("proposes");
    let r = runs
        .grant_plan(1, "g1", "SEALED", "o", 1)
        .expect("grants")
        .shown();
    let text = serde_json::to_string(&r).expect("serializes");
    assert!(!text.contains("SEALED"), "{text}");
}
