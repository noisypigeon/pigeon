//! The owner's digest; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §4.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

use backplane::approve::Waiting;
use backplane::mandates::{Mandates, State};
use backplane::tasks::{DigestInput, STALE, Tasks, digest};

const ADR: &str = ".claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md";

#[test]
fn an_empty_digest_asks_nothing_of_the_owner() {
    let d = digest(&DigestInput::default(), 100);
    assert!(d.is_empty());
    assert!(!d.needs_owner());
    assert_eq!(d.lines(), Vec::<String>::new());
}

#[test]
fn the_digest_names_what_waits_on_the_owner_and_what_merged_without_them() {
    let waiting = [
        Waiting {
            number: 62,
            operation: "change.merge".into(),
            title: "t".into(),
            path: "/approve/62?grant=x".into(),
            expires: STALE + 900,
        },
        Waiting {
            number: 61,
            operation: "change.approve".into(),
            title: "t".into(),
            path: "/approve/61?grant=y".into(),
            expires: STALE + 500,
        },
        Waiting {
            number: 60,
            operation: "change.merge".into(),
            title: "t".into(),
            path: "/approve/60?grant=z".into(),
            expires: 50,
        },
    ];
    let mut tasks = Tasks::default();
    tasks.add(
        "reconcile",
        70,
        Some(60),
        Some("s-a".into()),
        "fix: reconcile !60",
        0,
    );
    tasks.add("review", 71, None, None, "fresh", STALE);
    let mut mandates = Mandates::default();
    kept!(mandates.propose(ADR, "task", vec!["src/**".into()], false, None, None))
        .expect("proposes");
    kept!(mandates.offer(1, "tok", 0, 100)).expect("offers");
    kept!(mandates.grant("tok", "owner", 1)).expect("grants");
    kept!(mandates.pause_all("the deploy of abc failed"));
    let failed = [("abcdefabcdefabcdef".to_string(), "health".to_string())];
    let merged = [(59, "mandate:1".to_string())];
    let d = digest(
        &DigestInput {
            waiting: &waiting,
            tasks: Some(&tasks),
            mandates: &mandates.list,
            failed_deploys: &failed,
            merged_without_owner: &merged,
            mirror: None,
        },
        STALE + 10,
    );
    assert!(d.needs_owner());
    assert_eq!(
        d.approvals.iter().map(|a| a.0).collect::<Vec<_>>(),
        [61, 62],
        "live ones, soonest to expire first"
    );
    assert_eq!(d.stale_tasks.len(), 1, "only the task unclaimed two hours");
    assert_eq!(d.paused, [(1, "the deploy of abc failed".to_string())]);
    assert_eq!(mandates.list[0].state, State::Paused);
    let lines = d.lines();
    assert!(lines[0].contains("`change.approve` on change 61"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("unclaimed: task 1 (reconcile)"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l == "deploy failed: abcdefabcdef: health")
    );
    assert!(
        lines
            .iter()
            .any(|l| l == "merged without you: change 59, by mandate:1")
    );
}

#[test]
fn only_merges_without_the_owner_are_news_but_not_a_push() {
    let merged = [(59, "mandate:1".to_string())];
    let d = digest(
        &DigestInput {
            merged_without_owner: &merged,
            ..DigestInput::default()
        },
        0,
    );
    assert!(!d.is_empty());
    assert!(!d.needs_owner());
}

#[test]
fn a_mirror_behind_needs_the_owner() {
    let d = digest(
        &DigestInput {
            mirror: Some("mirror: GitHub has been behind for 25 hours"),
            ..DigestInput::default()
        },
        0,
    );
    assert!(d.needs_owner());
    assert_eq!(d.lines(), ["mirror: GitHub has been behind for 25 hours"]);
}
