//! The merge queue; see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use backplane::approvals::{Tier, Tiered, Tiers};
use backplane::changes::{Change, Check, Effect, Event, RESTACK, Relation, Restacked, State};
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
fn the_push_event_may_arrive_before_the_restack_report() {
    let mut c = approved();
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Pushed { sha: C.into() })
        .expect("pushed first");
    check(&mut c, C, Tier::Confirm);
    assert_eq!(c.state, State::Checked, "no report yet");
    c.apply(restacked(Relation::Disjoint, Some(C), true))
        .expect("restacked");
    assert_eq!(c.state, State::Approved);
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
