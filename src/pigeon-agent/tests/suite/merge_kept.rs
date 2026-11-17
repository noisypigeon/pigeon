//! A turn lost only because `main` moved is kept; see the 2026-11-15 amendment to
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.

use crate::harness::step;

use agent::control;
use agent::core::approval::tiers::{Tier, Tiered, Tiers};
use agent::core::change::queue::{HOLD, MergeQueue, PROTECT, Queued};
use agent::core::change::{
    Change, Check, Effect, Event, GATE, MERGE, RESTACK, Relation, Restacked, STALE_BASE, State,
    TRANSIENT_PUSH,
};
use step::{Apply, Queue};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";
const WINDOW: &str = "window:9";

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

/// Head `sha` bounded with one check, `fmt`, and a full set of `test`, then `fmt` passes.
fn check(c: &mut Change, sha: &str, merge: Tier) {
    let fmt = Check {
        name: "fmt".into(),
        argv: vec!["cargo".into(), "fmt".into()],
    };
    let test = Check {
        name: "test".into(),
        argv: vec!["cargo".into(), "test".into()],
    };
    let started = c
        .apply(Event::Bounded {
            sha: sha.into(),
            violations: vec![],
            checks: vec![fmt],
            full: vec![test],
            tiers: Some(tiers(merge)),
            gate: None,
            patch: None,
            own: None,
        })
        .expect("bounded");
    if !started.contains(&Effect::Start("fmt".into())) {
        return;
    }
    for e in [
        Event::Started {
            job: "fmt".into(),
            sha: sha.into(),
        },
        Event::Finished {
            job: "fmt".into(),
            sha: sha.into(),
            ok: true,
        },
    ] {
        c.apply(e).expect("fmt");
    }
}

/// Change `n` on head `A`, checked and approved by the window.
fn approved(n: u32) -> Change {
    let mut c = Change::propose(n, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: WINDOW.into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    check(&mut c, A, Tier::Code);
    c.apply(Event::Approve {
        by: WINDOW.into(),
        sha: A.into(),
        at: 5,
    })
    .expect("approved");
    c
}

fn restacked(relation: Relation, same_patch: bool) -> Event {
    let conflicting = relation == Relation::Conflicting;
    Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation,
            to: (!conflicting).then(|| C.into()),
            same_patch,
            conflicts: if conflicting {
                vec!["a/x.rs".into()]
            } else {
                vec![]
            },
            onto: MAIN.into(),
            context: vec![],
            messages: String::new(),
            landed: false,
        },
    }
}

/// What the Worker does with the queue after saving `after` over `before`: keep the turn, give
/// it back, or neither; the merge it starts next, if any.
fn saved(q: &mut MergeQueue, before: &Change, after: &Change, now: u64) -> Option<u32> {
    if control::keeps_turn(Some(before), after) {
        q.keep(after.number, now);
        return None;
    }
    if control::leaves_merge_queue(Some(before), after) {
        return q.done(after.number, now).map(|e| e.number);
    }
    None
}

/// Change `c`'s turn comes while it is behind: it restacks clean onto a `main` that changed its
/// own lines, so its approval doesn't carry, and its new head `C` is checked in full again. Each
/// step is saved as the Worker saves it; the merge the queue starts next, if any.
fn turn_lost_to_main(c: &mut Change, q: &mut MergeQueue, now: u64) -> Option<u32> {
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    let restack = c
        .apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    assert_eq!(restack, [Effect::Start(RESTACK.into())]);
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    assert_eq!((c.state, c.head.as_deref()), (State::Pushed, Some(C)));
    assert!(
        c.gate_why()
            .is_some_and(|w| w.contains("main changed its files")),
        "{:?}",
        c.gate_why()
    );
    let before = c.clone();
    check(c, C, Tier::Code);
    assert_eq!(c.state, State::Checked, "its approval didn't carry");
    saved(q, &before, c, now)
}

/// The window approves head `C` again, and its merge is asked as the Worker's `kept` asks it.
fn asked_again(c: &mut Change, q: &mut MergeQueue, now: u64) -> Result<Queued, String> {
    c.apply(Event::Approve {
        by: WINDOW.into(),
        sha: C.into(),
        at: now,
    })
    .expect("approved again");
    q.ask(c.number, WINDOW, now).1
}

/// The merge gate passes on head `sha`, and its merge job starts.
fn gated(c: &mut Change, sha: &str) {
    let gate = format!("{GATE}test");
    let started = c.apply(Event::Merge { by: WINDOW.into() }).expect("merge");
    assert_eq!(started, [Effect::Start(gate.clone())]);
    c.apply(Event::Started {
        job: gate.clone(),
        sha: sha.into(),
    })
    .expect("gate");
    let merge = c
        .apply(Event::Finished {
            job: gate,
            sha: sha.into(),
            ok: true,
        })
        .expect("gate passed");
    assert_eq!(merge, [Effect::Start(MERGE.into())]);
}

/// A queue with `ahead` merges waiting in front of change `n` and `behind` after it, change
/// 1000 merging.
fn queue(n: u32, ahead: u32, behind: u32) -> MergeQueue {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(1000, WINDOW, 0), Ok(Queued::Start));
    for i in 0..ahead {
        q.enqueue(1001 + i, WINDOW, 0).expect("ahead");
    }
    q.enqueue(n, WINDOW, 0).expect("its place");
    for i in 0..behind {
        q.enqueue(2000 + i, WINDOW, 0).expect("behind");
    }
    q
}

/// Merges the queue's head until change `n` takes the turn: the merges done.
fn until_turn(q: &mut MergeQueue, n: u32, now: u64) -> u32 {
    let mut merged = 0;
    while q.merging.as_ref().is_some_and(|e| e.number != n) {
        let at = q.merging.as_ref().map(|e| e.number).expect("merging");
        q.done(at, now);
        merged += 1;
    }
    merged
}

fn place(q: &MergeQueue, n: u32) -> Option<(usize, usize)> {
    let total = q.waiting.len() + usize::from(q.merging.is_some());
    if q.merging.as_ref().is_some_and(|e| e.number == n) {
        return Some((1, total));
    }
    q.waiting
        .iter()
        .position(|e| e.number == n)
        .map(|i| (i + 1 + usize::from(q.merging.is_some()), total))
}

#[test]
fn replay_546_keeps_its_turn_and_merges_on_it() {
    // !546 at 3/6 after its yield to !543 ended; 7 more were asked behind it by 22671.
    let (mut c, mut q) = (approved(546), queue(546, 1, 10));
    assert_eq!(place(&q, 546), Some((3, 13)));
    until_turn(&mut q, 546, 100);
    assert_eq!(place(&q, 546), Some((1, 11)));
    // Event 22671: checked, and its approval didn't carry.
    let next = turn_lost_to_main(&mut c, &mut q, 200);
    assert_eq!(next, None, "nothing else starts");
    assert_eq!(q.merging, None);
    assert_eq!(q.kept.get(&546).map(|k| k.misses), Some(1));
    // The window approves it at once (22674), and it takes the turn back.
    assert_eq!(asked_again(&mut c, &mut q, 213), Ok(Queued::Start));
    assert_eq!(place(&q, 546), Some((1, 11)), "not 13/14");
    assert_eq!(c.behind, None, "main hasn't moved");
    gated(&mut c, C);
    c.apply(Event::Merged {
        sha: C.into(),
        commit: Some(MAIN.into()),
    })
    .expect("merged");
    assert_eq!(c.state, State::Merged);
    assert_eq!(q.done(546, 900).map(|e| e.number), Some(2000));
    assert!(q.kept.is_empty(), "a merged change is forgotten");
}

#[test]
fn before_the_fix_546_went_to_the_tail() {
    // As before: the turn given back, the next merge started, and the ask queued at the tail.
    let (mut c, mut q) = (approved(546), queue(546, 1, 10));
    until_turn(&mut q, 546, 100);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    let before = c.clone();
    check(&mut c, C, Tier::Code);
    assert!(control::leaves_merge_queue(Some(&before), &c));
    q.done(546, 200);
    assert_eq!(q.enqueue(546, WINDOW, 213), Ok(Queued::Behind(10)));
    assert_eq!(place(&q, 546), Some((11, 11)));
}

#[test]
fn replay_549_is_first_again_when_the_owner_approves_it_later() {
    // !549 at 4/5; its approval didn't carry, and the window doesn't cover it this time.
    let (mut c, mut q) = (approved(549), queue(549, 2, 1));
    assert_eq!(place(&q, 549), Some((4, 5)));
    until_turn(&mut q, 549, 100);
    assert_eq!(turn_lost_to_main(&mut c, &mut q, 200), None);
    // The Worker's ask found no window: the queue moves on.
    assert_eq!(q.release(549, 201).map(|e| e.number), Some(2000));
    for n in 3000..3005 {
        q.enqueue(n, WINDOW, 210).expect("asked meanwhile");
    }
    // The owner approves it: first in line, not 10/10.
    assert_eq!(asked_again(&mut c, &mut q, 400), Ok(Queued::Behind(1)));
    assert_eq!(place(&q, 549), Some((2, 7)));
    assert_eq!(q.done(2000, 500).map(|e| e.number), Some(549));
}

#[test]
fn replay_481_is_protected_after_missing_its_turn_twice() {
    // !481 at 3/15, then 18/19 before the fix.
    let (mut c, mut q) = (approved(481), queue(481, 1, 12));
    assert_eq!(place(&q, 481), Some((3, 15)));
    until_turn(&mut q, 481, 100);
    assert_eq!(turn_lost_to_main(&mut c, &mut q, 200), None);
    // No window: the queue moves on, and 4 more are asked.
    assert_eq!(q.release(481, 201).map(|e| e.number), Some(2000));
    for n in 3000..3004 {
        q.enqueue(n, WINDOW, 210).expect("asked meanwhile");
    }
    assert_eq!(asked_again(&mut c, &mut q, 300), Ok(Queued::Behind(1)));
    assert_eq!(place(&q, 481), Some((2, 17)), "not 18/19");
    // 2000 merges and moves main; 481's turn comes, and main moved under it again.
    assert_eq!(q.done(2000, 400).map(|e| e.number), Some(481));
    let mut c2 = c.clone();
    c2.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c2.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c2.apply(Event::Restacked {
        sha: C.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch: false,
            conflicts: vec![],
            onto: MAIN.into(),
            context: vec![],
            messages: String::new(),
            landed: false,
        },
    })
    .expect("restacked");
    let before = c2.clone();
    check(&mut c2, B, Tier::Code);
    assert_eq!(saved(&mut q, &before, &c2, 500), None);
    assert_eq!(q.kept.get(&481).map(|k| k.misses), Some(PROTECT));
    // Protected: nothing else starts, whoever asks, until it is asked again.
    assert_eq!(q.protected(501), Some(481));
    assert_eq!(q.advance(501), None);
    assert_eq!(q.enqueue(3100, WINDOW, 502), Ok(Queued::Behind(16)));
    assert_eq!(q.merging, None);
    c2.apply(Event::Approve {
        by: "owner".into(),
        sha: B.into(),
        at: 900,
    })
    .expect("owner approves");
    assert_eq!(q.ask(481, "owner", 900).1, Ok(Queued::Start));
    gated(&mut c2, B);
}

#[test]
fn a_protected_turn_lapses_after_its_hold() {
    let mut q = queue(7, 0, 1);
    q.done(1000, 0);
    q.keep(7, 10);
    q.keep(7, 20);
    assert_eq!(q.protected(20 + HOLD - 1), Some(7));
    assert_eq!(q.advance(20 + HOLD - 1), None);
    assert_eq!(q.protected(20 + HOLD), None);
    assert_eq!(q.advance(20 + HOLD).map(|e| e.number), Some(2000));
}

#[test]
fn replay_543_a_merge_refused_on_a_stale_base_restacks_and_keeps_its_turn() {
    // !543's merge job: "main has moved past change/543-standing-auto's base", before its
    // `behind` arrived (event 21520).
    let mut c = approved(543);
    gated(&mut c, A);
    c.apply(Event::Started {
        job: MERGE.into(),
        sha: A.into(),
    })
    .expect("merge started");
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: format!(
            "main {STALE_BASE} change/543-standing-auto's base; rebase onto main and push"
        ),
    };
    let effects = c.apply(refused.clone()).expect("refused");
    assert_eq!(effects, [Effect::Start(RESTACK.into())]);
    assert_eq!(c.merge_after.as_deref(), Some(WINDOW));
    assert!(agent::core::change::queue::holds_turn(&c));
    assert_eq!(control::merge_ended(&refused, &c), None, "the turn is kept");
    // A disjoint restack of the same patch carries its approval, and it merges on its turn.
    c.apply(Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Disjoint,
            to: Some(C.into()),
            same_patch: true,
            conflicts: vec![],
            onto: MAIN.into(),
            context: vec![],
            messages: String::new(),
            landed: false,
        },
    })
    .expect("restacked");
    check(&mut c, C, Tier::Code);
    assert_eq!(c.state, State::Approved);
}

#[test]
fn a_merge_refused_for_another_reason_gives_its_turn_back() {
    let mut c = approved(9);
    gated(&mut c, A);
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: "no".into(),
    };
    c.apply(refused.clone()).expect("refused");
    assert_eq!(control::merge_ended(&refused, &c), Some(None));
}

#[test]
fn a_merge_push_refused_for_now_merges_again_and_keeps_its_turn() {
    // !842's merge job: its push to main met `artifacts_git_receive_pack_busy` on every try.
    let mut c = approved(842);
    gated(&mut c, A);
    c.apply(Event::Started {
        job: MERGE.into(),
        sha: A.into(),
    })
    .expect("merge started");
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: format!(
            "git push: {C}:refs/heads/main {TRANSIENT_PUSH} 5 times: remote: artifacts_git_receive_pack_busy"
        ),
    };
    let effects = c.apply(refused.clone()).expect("refused");
    assert_eq!(
        effects,
        [Effect::Start(MERGE.into())],
        "no gate, no restack"
    );
    assert!(agent::core::change::queue::holds_turn(&c));
    assert_eq!(control::merge_ended(&refused, &c), None, "the turn is kept");
    c.apply(Event::Started {
        job: MERGE.into(),
        sha: A.into(),
    })
    .expect("merge started again");
    let merged = Event::Merged {
        sha: A.into(),
        commit: Some(MAIN.into()),
    };
    c.apply(merged.clone()).expect("merged");
    assert_eq!(c.state, State::Merged);
    assert_eq!(control::merge_ended(&merged, &c), Some(Some(MAIN.into())));
}

#[test]
fn a_failed_check_after_the_restack_loses_the_turn() {
    let (mut c, mut q) = (approved(10), queue(10, 0, 2));
    until_turn(&mut q, 10, 100);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    c.apply(Event::Bounded {
        sha: C.into(),
        violations: vec![],
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        full: vec![],
        tiers: Some(tiers(Tier::Code)),
        gate: None,
        patch: None,
        own: None,
    })
    .expect("bounded");
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: C.into(),
    })
    .expect("started");
    let before = c.clone();
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: C.into(),
        ok: false,
    })
    .expect("failed");
    assert_eq!(c.state, State::Failed);
    assert!(!control::keeps_turn(Some(&before), &c));
    saved(&mut q, &before, &c, 200);
    assert!(q.kept.is_empty());
}

#[test]
fn a_conflict_found_by_the_restack_loses_the_turn() {
    let (mut c, mut q) = (approved(11), queue(11, 0, 1));
    until_turn(&mut q, 11, 100);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    let before = c.clone();
    c.apply(restacked(Relation::Conflicting, false))
        .expect("conflict");
    assert!(!control::keeps_turn(Some(&before), &c));
    assert_eq!(saved(&mut q, &before, &c, 200), Some(2000));
}

#[test]
fn a_sessions_own_push_while_parked_does_not_keep_the_turn() {
    let (mut c, mut q) = (approved(12), queue(12, 0, 1));
    until_turn(&mut q, 12, 100);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    assert_eq!(c.restacked_for.as_deref(), Some(WINDOW));
    c.apply(Event::Pushed { sha: B.into() })
        .expect("the session pushes");
    assert_eq!(c.restacked_for, None);
    let before = c.clone();
    check(&mut c, B, Tier::Code);
    assert!(!control::keeps_turn(Some(&before), &c));
}

#[test]
fn a_yielded_change_keeps_its_place_beside_a_kept_one() {
    let mut q = queue(20, 0, 0);
    q.yielded.insert(21, 20);
    assert!(matches!(
        q.enqueue(21, WINDOW, 0),
        Ok(Queued::Yielded { place: 3, to: 20 })
    ));
    assert_eq!(q.done(1000, 1).map(|e| e.number), Some(20));
    q.keep(20, 2);
    assert_eq!(q.advance(3), None, "21 still yields to 20");
    assert_eq!(q.ask(20, WINDOW, 4).1, Ok(Queued::Start));
    assert_eq!(place(&q, 21), Some((2, 2)));
}
