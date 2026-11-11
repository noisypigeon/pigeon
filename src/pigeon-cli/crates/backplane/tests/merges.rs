//! The merge queue; see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use backplane::approvals::{Tier, Tiered, Tiers};
use backplane::changes::{
    Change, Check, Effect, Event, GATE, MERGE, RESTACK, Relation, Restacked, State,
};
use backplane::control;
use backplane::merges::{MergeQueue, Queued, STUCK};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn merges_run_one_at_a_time_in_the_order_asked() {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(60, "owner", 1), Ok(Queued::Start));
    assert_eq!(q.enqueue(61, "mandate:2", 2), Ok(Queued::Behind(1)));
    assert_eq!(q.enqueue(62, "owner", 3), Ok(Queued::Behind(2)));
    assert!(q.enqueue(61, "owner", 4).is_err(), "already waiting");
    assert!(q.enqueue(60, "owner", 4).is_err(), "already merging");
    let next = q.done(60, 10).expect("61 is next");
    assert_eq!(
        (next.number, next.by.as_str(), next.at),
        (61, "mandate:2", 10)
    );
    assert_eq!(q.done(99, 11), None, "an unknown change starts nothing");
    assert_eq!(q.merging.as_ref().map(|e| e.number), Some(61));
    assert_eq!(q.done(61, 12).map(|e| e.number), Some(62));
    assert_eq!(q.done(62, 13), None);
    assert_eq!(q, MergeQueue::default(), "empty again");
}

#[test]
fn a_change_closed_while_waiting_leaves_the_queue() {
    let mut q = MergeQueue::default();
    q.enqueue(60, "owner", 1).expect("starts");
    q.enqueue(61, "owner", 2).expect("waits");
    q.enqueue(62, "owner", 3).expect("waits");
    assert_eq!(q.done(61, 4), None, "60 still merges");
    assert_eq!(q.done(60, 5).map(|e| e.number), Some(62));
}

#[test]
fn a_stuck_merge_is_moved_past() {
    let mut q = MergeQueue::default();
    q.enqueue(60, "owner", 100).expect("starts");
    q.enqueue(61, "owner", 101).expect("waits");
    assert_eq!(q.unstick(100 + STUCK), None, "not yet");
    assert_eq!(q.unstick(101 + STUCK).map(|e| e.number), Some(61));
}

#[test]
fn a_change_is_marked_behind_main_until_its_next_push() {
    let mut c = Change::propose(60, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    assert!(
        c.apply(Event::Behind { main: B.into() }).is_err(),
        "no head yet"
    );
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("approves");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Behind { main: B.into() }).expect("behind");
    assert_eq!(c.behind.as_deref(), Some(B));
    assert_eq!(c.state, State::Pushed, "its state is unchanged");
    c.apply(Event::Pushed { sha: B.into() }).expect("pushed");
    assert_eq!(c.behind, None, "a new head is rebased");
}

const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";

fn tiers(merge: Tier) -> Tiers {
    let t = |tier| Tiered {
        tier,
        rule: "routine".into(),
    };
    Tiers {
        authorize: t(Tier::Confirm),
        merge: t(merge),
        close: t(Tier::Confirm),
    }
}

/// Checks head `sha`: bound with `merge`'s tier, then its one check passes.
fn check(c: &mut Change, sha: &str, merge: Tier) {
    c.apply(Event::Bounded {
        sha: sha.into(),
        violations: vec![],
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        full: vec![],
        tiers: Some(tiers(merge)),
    })
    .expect("bounded");
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: sha.into(),
    })
    .expect("started");
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: sha.into(),
        ok: true,
    })
    .expect("finished");
}

fn approved() -> Change {
    let mut c = Change::propose(60, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    check(&mut c, A, Tier::Confirm);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 5,
    })
    .expect("approved");
    c
}

fn restacked(relation: Relation, to: Option<&str>, same_patch: bool) -> Event {
    Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation,
            to: to.map(str::to_string),
            same_patch,
            conflicts: if relation == Relation::Conflicting {
                vec!["a/x.rs".into()]
            } else {
                vec![]
            },
            onto: MAIN.into(),
            context: vec![],
            messages: String::new(),
        },
    }
}

#[test]
fn an_approved_disjoint_head_is_restacked_and_keeps_its_approval() {
    let mut c = approved();
    let effects = c
        .apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    assert_eq!(effects, [Effect::Start(RESTACK.into())]);
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    assert_eq!((c.state, c.head.as_deref()), (State::Pushed, Some(C)));
    assert_eq!(c.behind, None);
    assert_eq!(c.approval, None, "until its checks pass again");
    check(&mut c, C, Tier::Confirm);
    assert_eq!(c.state, State::Approved);
    let a = c.approval.as_ref().expect("carried");
    assert_eq!(
        (a.by.as_str(), a.sha.as_deref(), a.at),
        ("owner", Some(C), 5)
    );
    assert_eq!(c.carried, None);
}

#[test]
fn a_push_during_a_restack_is_held_until_the_restack_reports() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    assert_eq!(
        c.apply(Event::Pushed { sha: C.into() }).expect("held"),
        [],
        "the restack's own push, reported before the job"
    );
    assert_eq!((c.state, c.head.as_deref()), (State::Approved, Some(A)));
    assert_eq!(c.held.as_deref(), Some(C));
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    assert_eq!(
        (c.state, c.head.as_deref(), c.held.as_deref()),
        (State::Pushed, Some(C), None)
    );
    check(&mut c, C, Tier::Confirm);
    assert_eq!(c.state, State::Approved, "the approval carried");
}

#[test]
fn a_sessions_push_during_a_failed_restack_becomes_the_head() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Started {
        job: RESTACK.into(),
        sha: A.into(),
    })
    .expect("started");
    c.apply(Event::Pushed { sha: B.into() }).expect("held");
    let effects = c
        .apply(Event::Finished {
            job: RESTACK.into(),
            sha: A.into(),
            ok: false,
        })
        .expect("failed");
    assert!(
        effects.contains(&Effect::Start("bound".into())),
        "{effects:?}"
    );
    assert_eq!((c.state, c.head.as_deref()), (State::Pushed, Some(B)));
    assert_eq!((c.carried.clone(), c.held.clone()), (None, None));
}

#[test]
fn an_approval_is_not_carried_when_the_patch_or_tier_changed() {
    for (relation, same, tier) in [
        (Relation::Clean, true, Tier::Confirm),
        (Relation::Disjoint, false, Tier::Confirm),
        (Relation::Disjoint, true, Tier::Code),
    ] {
        let mut c = approved();
        c.apply(Event::Behind { main: MAIN.into() })
            .expect("behind");
        c.apply(restacked(relation, Some(C), same))
            .expect("restacked");
        check(&mut c, C, tier);
        assert_eq!(c.state, State::Checked, "{relation:?} {same} {tier:?}");
        assert_eq!(c.approval, None);
    }
}

#[test]
fn a_conflict_is_reported_and_the_head_kept() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(restacked(Relation::Conflicting, None, false))
        .expect("restacked");
    assert_eq!((c.state, c.head.as_deref()), (State::Approved, Some(A)));
    assert!(c.violations[0].contains("a/x.rs"), "{:?}", c.violations);
    assert_eq!(c.behind.as_deref(), Some(MAIN));
}

#[test]
fn only_a_checked_or_approved_head_is_restacked() {
    let mut c = Change::propose(60, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    assert_eq!(
        c.apply(Event::Behind { main: MAIN.into() })
            .expect("behind"),
        []
    );
    assert_eq!(
        c.apply(restacked(Relation::Current, None, true))
            .expect("current"),
        []
    );
    assert_eq!(c.behind, None);
}

/// Bounds head `sha` with `merge`'s tier and starts its one check; the check's finish is left
/// to the caller.
fn bound(c: &mut Change, sha: &str, merge: Tier) {
    c.apply(Event::Bounded {
        sha: sha.into(),
        violations: vec![],
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        full: vec![],
        tiers: Some(tiers(merge)),
    })
    .expect("bounded");
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: sha.into(),
    })
    .expect("started");
}

fn finish(c: &mut Change, sha: &str) -> Vec<Effect> {
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: sha.into(),
        ok: true,
    })
    .expect("finished")
}

#[test]
fn a_merge_asked_for_while_restacking_starts_once_the_approval_carries() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    let asked = c.apply(Event::Merge { by: "owner".into() }).expect("merge");
    assert_eq!(asked, [], "it waits for the restack");
    assert_eq!(c.merge_after.as_deref(), Some("owner"));
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    bound(&mut c, C, Tier::Confirm);
    assert_eq!(finish(&mut c, C), [Effect::Start(MERGE.into())]);
    assert_eq!(c.state, State::Approved);
    assert_eq!(c.merge_after, None);
    assert!(!backplane::control::merge_dropped(&c), "its merge runs");
}

#[test]
fn a_merge_asked_for_while_a_current_restack_runs_starts_when_it_reports() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: "owner".into() }).expect("merge");
    let effects = c
        .apply(restacked(Relation::Current, None, true))
        .expect("current");
    assert_eq!(effects, [Effect::Start(MERGE.into())]);
}

#[test]
fn a_merge_whose_approval_did_not_carry_gives_up_its_turn() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: "owner".into() }).expect("merge");
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    bound(&mut c, C, Tier::Code);
    assert_eq!(finish(&mut c, C), []);
    assert_eq!((c.state, c.merge_after.as_deref()), (State::Checked, None));
    assert!(backplane::control::merge_dropped(&c));

    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: "owner".into() }).expect("merge");
    c.apply(restacked(Relation::Conflicting, None, false))
        .expect("conflict");
    assert_eq!(c.merge_after, None);
    assert!(backplane::control::merge_dropped(&c));
}

#[test]
fn a_merge_on_a_checked_unapproved_head_is_refused_not_parked() {
    // !138: restacked, then checked with no tiers, so its approval never carried.
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    c.apply(Event::Bounded {
        sha: C.into(),
        violations: vec![],
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        full: vec![],
        tiers: None,
    })
    .expect("bounded");
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: C.into(),
    })
    .expect("started");
    assert_eq!(finish(&mut c, C), []);
    assert_eq!(c.state, State::Checked);
    assert_eq!(c.carried, None, "the carry is decided");

    let refused = c
        .apply(Event::Merge { by: "owner".into() })
        .expect_err("an unapproved head isn't merged");
    assert!(
        format!("{refused:?}").contains("only an approved head"),
        "{refused:?}"
    );
    assert_eq!(
        c.merge_after, None,
        "nothing parked, so the queue's turn is released"
    );

    c.apply(Event::Approve {
        by: "owner".into(),
        sha: C.into(),
        at: 9,
    })
    .expect("approved");
    assert_eq!(
        c.apply(Event::Merge { by: "owner".into() }).expect("merge"),
        [Effect::Start(MERGE.into())]
    );
}

#[test]
fn a_merge_whose_container_is_lost_on_every_attempt_releases_its_turn() {
    use backplane::control::merge_ended;
    use backplane::runs::MAX_ATTEMPTS;

    // !142: its merge job's Container was lost three times, and the queue kept its turn.
    let mut c = approved();
    assert_eq!(
        c.apply(Event::Merge { by: "owner".into() }).expect("merge"),
        [Effect::Start(MERGE.into())]
    );
    for n in 1..=MAX_ATTEMPTS {
        c.apply(Event::Started {
            job: MERGE.into(),
            sha: A.into(),
        })
        .expect("started");
        let lost = Event::Lost {
            job: MERGE.into(),
            sha: A.into(),
        };
        c.apply(lost.clone()).expect("lost");
        if n < MAX_ATTEMPTS {
            assert_eq!(merge_ended(&lost, &c), None, "attempt {n} is retried");
        } else {
            assert_eq!(merge_ended(&lost, &c), Some(None), "the last loss ends it");
        }
    }

    let other = Event::Lost {
        job: "fmt".into(),
        sha: A.into(),
    };
    assert_eq!(merge_ended(&other, &c), None, "only the merge job's loss");
    assert_eq!(
        merge_ended(
            &Event::Merged {
                sha: A.into(),
                commit: Some(B.into()),
            },
            &c
        ),
        Some(Some(B.into()))
    );
    assert_eq!(
        merge_ended(
            &Event::MergeFailed {
                sha: A.into(),
                reason: "x".into(),
            },
            &c
        ),
        Some(None)
    );
}

fn named(n: &str) -> Check {
    Check {
        name: n.into(),
        argv: vec!["cargo".into(), n.into()],
    }
}

/// An approved change on head `A` whose checks were `checks`, all passed, and whose full set is
/// `full`.
fn approved_with(checks: &[&str], full: &[&str]) -> Change {
    let mut c = Change::propose(70, "g", "feat(g): h", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Bounded {
        sha: A.into(),
        violations: vec![],
        checks: checks.iter().map(|n| named(n)).collect(),
        full: full.iter().map(|n| named(n)).collect(),
        tiers: Some(tiers(Tier::Confirm)),
    })
    .expect("bounded");
    for n in checks {
        c.apply(Event::Finished {
            job: (*n).into(),
            sha: A.into(),
            ok: true,
        })
        .expect("finished");
    }
    assert_eq!(c.state, State::Checked);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 5,
    })
    .expect("approved");
    c
}

fn gate(n: &str) -> String {
    format!("{GATE}{n}")
}

#[test]
fn the_merge_gate_runs_the_full_checks_not_yet_passed_then_merges() {
    let mut c = approved_with(&["fmt"], &["fmt", "clippy", "test"]);
    let effects = c.apply(Event::Merge { by: "owner".into() }).expect("gates");
    assert_eq!(
        effects,
        [Effect::Start(gate("clippy")), Effect::Start(gate("test"))],
        "fmt already passed on this head"
    );
    assert_eq!(c.merge_after.as_deref(), Some("owner"));
    assert!(
        !control::merge_dropped(&c),
        "the turn is held while the gate runs"
    );
    let none = c
        .apply(Event::Finished {
            job: gate("clippy"),
            sha: A.into(),
            ok: true,
        })
        .expect("finished");
    assert!(none.is_empty(), "one still runs");
    let merge = c
        .apply(Event::Finished {
            job: gate("test"),
            sha: A.into(),
            ok: true,
        })
        .expect("finished");
    assert_eq!(merge, [Effect::Start(MERGE.into())]);
    assert_eq!(c.state, State::Approved);
    assert_eq!(c.merge_after, None);
}

#[test]
fn a_failed_gate_fails_the_change_and_releases_the_merge_turn() {
    let mut c = approved_with(&["fmt"], &["fmt", "clippy", "test"]);
    c.apply(Event::Merge { by: "owner".into() }).expect("gates");
    let effects = c
        .apply(Event::Finished {
            job: gate("test"),
            sha: A.into(),
            ok: false,
        })
        .expect("finished");
    assert_eq!(effects, [Effect::Cancel(vec![gate("clippy")])]);
    assert_eq!(c.state, State::Failed);
    assert!(
        c.violations
            .iter()
            .any(|v| v.contains("merge gate") && v.contains("test")),
        "{:?}",
        c.violations
    );
    assert_eq!(c.merge_after, None);
    assert!(control::merge_dropped(&c), "the queue moves on");
}

#[test]
fn a_head_without_rust_or_already_fully_checked_merges_at_once() {
    let mut docs = approved_with(&["fmt"], &[]);
    assert_eq!(
        docs.apply(Event::Merge { by: "owner".into() })
            .expect("merges"),
        [Effect::Start(MERGE.into())]
    );
    let mut whole = approved_with(&["fmt", "test"], &["fmt", "test"]);
    assert_eq!(
        whole
            .apply(Event::Merge { by: "owner".into() })
            .expect("merges"),
        [Effect::Start(MERGE.into())],
        "its own checks were the full set"
    );
}

#[test]
fn a_new_head_forgets_the_old_heads_full_set() {
    let mut c = approved_with(&["fmt"], &["fmt", "test"]);
    assert_eq!(c.full.len(), 2);
    c.apply(Event::Pushed { sha: B.into() }).expect("pushed");
    assert!(c.full.is_empty(), "the bound job reports the new head's");
}
