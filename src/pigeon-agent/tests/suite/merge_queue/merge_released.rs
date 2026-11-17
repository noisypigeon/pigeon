//! A head that fails, or a holder that yields, gives its turn back; see the 2026-11-16 amendment
//! to `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.

use crate::harness::step;

use agent::control;
use agent::core::approval::tiers::{Tier, Tiered, Tiers};
use agent::core::change::queue::{self as merges, Back, MergeQueue, Queued};
use agent::core::change::stage::{self, Needs};
use agent::core::change::{
    Change, Check, Effect, Event, GATE, MERGE, RESTACK, Relation, Restacked, State,
};
use step::{Apply, Queue};
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const F: &str = "ffffffffffffffffffffffffffffffffffffffff";
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
            carries: None,
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

/// Change `c`'s turn comes while it is behind; its clean restack's head `C` is bounded with
/// `fmt` and a full gate whose whole-suite run starts beside it: the gate job's name.
fn restacked_with_early_gate(c: &mut Change) -> String {
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    let suite = Check {
        name: "test".into(),
        argv: vec!["cargo".into(), "test".into(), "--workspace".into()],
    };
    let started = c
        .apply(Event::Bounded {
            sha: C.into(),
            violations: vec![],
            checks: vec![Check {
                name: "fmt".into(),
                argv: vec![],
            }],
            full: vec![suite],
            tiers: Some(tiers(Tier::Code)),
            gate: None,
            patch: None,
            own: None,
        })
        .expect("bounded");
    let gate = format!("{GATE}test");
    assert!(
        started.contains(&Effect::Start(gate.clone())),
        "{started:?}"
    );
    for job in ["fmt", gate.as_str()] {
        c.apply(Event::Started {
            job: job.into(),
            sha: C.into(),
        })
        .expect("started");
    }
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: C.into(),
        ok: true,
    })
    .expect("fmt");
    assert_eq!(c.state, State::Bounded);
    assert!(c.merge_after.is_some(), "its merge is parked");
    gate
}

#[test]
fn replay_675_a_gate_failed_after_the_restack_gives_the_turn_back() {
    // !675 holds the turn; !679, the fix for its failure, waits behind it.
    let (mut c, mut q) = (approved(675), queue(675, 0, 0));
    q.enqueue(679, WINDOW, 0).expect("!679 waits");
    until_turn(&mut q, 675, 100);
    let gate = restacked_with_early_gate(&mut c);
    let before = c.clone();
    c.apply(Event::Finished {
        job: gate,
        sha: C.into(),
        ok: false,
    })
    .expect("the gate fails");
    assert_eq!(c.state, State::Failed);
    assert_eq!(c.merge_after, None, "its parked merge goes back");
    assert!(!merges::holds_turn(&c));
    assert!(!control::keeps_turn(Some(&before), &c));
    assert_eq!(control::release_reason(&c), "its head failed gate: test");
    assert_eq!(saved(&mut q, &before, &c, 200), Some(679), "!679 starts");
    assert_eq!(place(&q, 675), None);
    let stage = stage::of(&c, &stage::Context::default());
    assert_eq!(
        (stage.stage.as_str(), stage.needs),
        ("failed", Needs::Agent)
    );
    // A later ask queues it as any change.
    assert_eq!(q.enqueue(675, WINDOW, 300), Ok(Queued::Behind(1)));
}

#[test]
fn a_holder_that_yields_gives_its_turn_back_and_keeps_its_place() {
    let (mut c, mut q) = (approved(675), queue(675, 0, 1));
    until_turn(&mut q, 675, 100);
    let gate = restacked_with_early_gate(&mut c);
    let before = c.clone();
    let mut c = c
        .yield_to(673, "agent", "!673 fixes main", 150)
        .expect("yields");
    assert_eq!(c.merge_after, None);
    assert!(control::leaves_merge_queue(Some(&before), &c));
    assert!(!control::keeps_turn(Some(&before), &c));
    assert!(merges::keeps_place(&c));
    assert_eq!(control::release_reason(&c), "it yields to !673");
    q.yielded.insert(675, 673);
    let (mut q, next) = merges::yielded_back(&q, 675, 160);
    assert_eq!(next.map(|e| e.number), Some(2000), "the next starts");
    assert_eq!(place(&q, 675), Some((2, 2)), "first in line");
    // Its gate passing now starts no merge without a turn.
    let started = c
        .apply(Event::Finished {
            job: gate,
            sha: C.into(),
            ok: true,
        })
        .expect("gate");
    assert!(!started.contains(&Effect::Start(MERGE.into())));
    // Its yield ends once 2000 merges: it is next.
    q.yielded.clear();
    assert_eq!(q.done(2000, 300).map(|e| e.number), Some(675));
}

#[test]
fn a_failed_holder_that_yields_leaves_the_queue() {
    let mut c = approved(675);
    let gate = restacked_with_early_gate(&mut c);
    c.apply(Event::Finished {
        job: gate,
        sha: C.into(),
        ok: false,
    })
    .expect("the gate fails");
    let c = c.yield_to(673, "agent", "r", 150).expect("yields");
    assert!(!merges::keeps_place(&c));
}

#[test]
fn a_change_being_merged_can_not_yield() {
    let mut c = approved(9);
    gated(&mut c, A);
    assert!(c.yield_to(673, "agent", "r", 1).is_err());
}

#[test]
fn a_turn_lost_to_main_is_still_kept() {
    let (mut c, mut q) = (approved(546), queue(546, 0, 1));
    until_turn(&mut q, 546, 100);
    assert_eq!(turn_lost_to_main(&mut c, &mut q, 200), None);
    assert_eq!(q.kept.get(&546).map(|k| k.misses), Some(1));
    assert!(!merges::keeps_place(&c));
}

#[test]
fn a_failed_check_after_the_restack_gives_the_turn_back() {
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
    assert_eq!(
        saved(&mut q, &before, &c, 200),
        Some(2000),
        "its turn goes back"
    );
    assert!(q.kept.is_empty());
}

#[test]
fn replay_674_a_fix_pushed_after_its_gate_failed_holds_no_turn() {
    // !674's gate failed after its restack, then a fix was pushed (cefd256) while it held the turn.
    let (mut c, mut q) = (approved(674), queue(674, 0, 0));
    q.enqueue(680, WINDOW, 0).expect("!680 waits");
    until_turn(&mut q, 674, 100);
    let gate = restacked_with_early_gate(&mut c);
    let before = c.clone();
    c.apply(Event::Finished {
        job: gate,
        sha: C.into(),
        ok: false,
    })
    .expect("the gate fails");
    assert_eq!(saved(&mut q, &before, &c, 200), Some(680), "!680 starts");
    let before = c.clone();
    c.apply(Event::Pushed {
        sha: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
    })
    .expect("the fix is pushed");
    assert_eq!(c.state, State::Pushed);
    assert_eq!(c.merge_after, None);
    assert!(!merges::holds_turn(&c));
    assert_eq!(saved(&mut q, &before, &c, 300), None);
    assert_eq!(place(&q, 674), None);
}

/// Head `sha` of `c` bounded with patch `patch`, one check `fmt`, a full set of `test`, and
/// `fmt` passed.
fn bound_with_patch(c: &mut Change, sha: &str, patch: &str) {
    let fmt = Check {
        name: "fmt".into(),
        argv: vec![],
    };
    let test = Check {
        name: "test".into(),
        argv: vec!["cargo".into(), "test".into()],
    };
    c.apply(Event::Bounded {
        sha: sha.into(),
        violations: vec![],
        checks: vec![fmt],
        full: vec![test],
        tiers: Some(tiers(Tier::Code)),
        gate: None,
        patch: Some(patch.into()),
        own: None,
    })
    .expect("bounded");
}

/// Change `n`, approved on head `A` with patch `p1`, holding the turn with its merge parked on
/// its gate.
fn parked_holder(n: u32) -> Change {
    let mut c = Change::propose(n, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: WINDOW.into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    bound_with_patch(&mut c, A, "p1");
    for e in [
        Event::Started {
            job: "fmt".into(),
            sha: A.into(),
        },
        Event::Finished {
            job: "fmt".into(),
            sha: A.into(),
            ok: true,
        },
        Event::Approve {
            by: WINDOW.into(),
            sha: A.into(),
            at: 5,
        },
    ] {
        c.apply(e).expect("approved");
    }
    let gate = c
        .apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    assert_eq!(gate, [Effect::Start(format!("{GATE}test"))]);
    assert!(merges::holds_turn(&c));
    c
}

#[test]
fn a_new_patch_pushed_onto_the_holder_gives_its_turn_back_at_bound() {
    let mut q = queue(674, 0, 1);
    until_turn(&mut q, 674, 100);
    let mut c = parked_holder(674);
    c.apply(Event::Pushed { sha: C.into() }).expect("the fix");
    assert!(c.merge_after.is_some(), "kept until its patch is known");
    let before = c.clone();
    bound_with_patch(&mut c, C, "p2");
    assert_eq!(c.merge_after, None);
    assert!(!control::keeps_turn(Some(&before), &c));
    assert_eq!(
        saved(&mut q, &before, &c, 200),
        Some(2000),
        "the next starts"
    );
}

#[test]
fn the_same_patch_pushed_onto_the_holder_keeps_its_turn() {
    let mut q = queue(674, 0, 1);
    until_turn(&mut q, 674, 100);
    let mut c = parked_holder(674);
    c.apply(Event::Pushed { sha: C.into() }).expect("a reword");
    let before = c.clone();
    bound_with_patch(&mut c, C, "p1");
    assert_eq!(c.merge_after.as_deref(), Some(WINDOW));
    assert!(merges::holds_turn(&c));
    assert_eq!(saved(&mut q, &before, &c, 200), None);
    assert_eq!(place(&q, 674), Some((1, 2)));
}

/// Head `sha` of `c` bounded with patch `patch` and own lines `own`, one check `fmt`, a full set
/// of `test`, and `fmt` passed.
fn bound_own(c: &mut Change, sha: &str, patch: &str, own: &str) {
    let fmt = Check {
        name: "fmt".into(),
        argv: vec![],
    };
    let test = Check {
        name: "test".into(),
        argv: vec!["cargo".into(), "test".into()],
    };
    c.apply(Event::Bounded {
        sha: sha.into(),
        violations: vec![],
        checks: vec![fmt],
        full: vec![test],
        tiers: Some(tiers(Tier::Code)),
        gate: None,
        patch: Some(patch.into()),
        own: Some(own.into()),
    })
    .expect("bounded");
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

/// Head `sha` of `c`, checked with own lines `own`, approved by the window at `at`.
fn approved_with_own(c: &mut Change, sha: &str, own: &str, at: u64) {
    bound_own(c, sha, "p", own);
    c.apply(Event::Approve {
        by: WINDOW.into(),
        sha: sha.into(),
        at,
    })
    .expect("approved");
}

/// !821 (`c`) moved to the front from 18/18, its head failing its gate after the restack at its
/// turn: the queue once it gave its turn back, 1001 merging and 1002 to 1016 waiting.
fn fronted_then_failed(c: &mut Change) -> MergeQueue {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(1000, WINDOW, 0), Ok(Queued::Start));
    for i in 1..=16 {
        q.enqueue(1000 + i, WINDOW, 0).expect("ahead");
    }
    let (q, queued) = merges::enqueue_back(&q, 821, WINDOW, &Back::of(c), 0).expect("queued");
    assert_eq!(queued, Queued::Behind(17));
    let (mut q, left) = merges::front(&q, 821).expect("fronted");
    assert_eq!(left, (18, 18));
    assert_eq!(until_turn(&mut q, 821, 100), 1);
    let gate = restacked_with_early_gate(c);
    let before = c.clone();
    c.apply(Event::Finished {
        job: gate,
        sha: C.into(),
        ok: false,
    })
    .expect("clippy fails");
    assert!(control::leaves_merge_queue(Some(&before), c));
    assert_eq!(merges::failed_at(c), Some(1));
    let (q, next) = merges::left(&q, 821, merges::failed_at(c), 200);
    assert_eq!(next.map(|e| e.number), Some(1001), "the next starts");
    assert_eq!(place(&q, 821), None);
    q
}

#[test]
fn replay_821_its_fix_after_a_failed_head_takes_the_front_back() {
    // !821 was fronted (event 36968), failed clippy after its restack at the head and gave its
    // turn back (37019); its one-line fix then queued it 18/18.
    let mut c = approved(821);
    let q = fronted_then_failed(&mut c);
    c.apply(Event::Pushed { sha: E.into() }).expect("the fix");
    approved_with_own(&mut c, E, "fixed", 800);
    assert_eq!(c.pushes, 2);
    let (q, queued) =
        merges::enqueue_back(&q, 821, WINDOW, &Back::of(&c), 900).expect("asked again");
    assert_eq!(queued, Queued::Behind(1), "first in line, not 17/17");
    assert_eq!(place(&q, 821), Some((2, 17)));
    assert!(q.returning.is_empty(), "a kept place is used once");
    assert!(q.marks.get(&821).is_some_and(|m| m.returned && m.fronted));
}

#[test]
fn a_kept_place_is_used_once_so_a_second_failure_queues_its_fix_at_the_back() {
    let mut c = approved(821);
    let q = fronted_then_failed(&mut c);
    let why = q.returning.get(&821).and_then(|r| {
        r.returns(
            &Back {
                own: None,
                pushes: 2,
            },
            900,
        )
    });
    assert_eq!(why, Some("it is the first push since its head failed"));
    c.apply(Event::Pushed { sha: E.into() }).expect("the fix");
    approved_with_own(&mut c, E, "fixed", 800);
    let (mut q, queued) =
        merges::enqueue_back(&q, 821, WINDOW, &Back::of(&c), 900).expect("asked again");
    assert_eq!(queued, Queued::Behind(1), "back at the front");
    assert_eq!(until_turn(&mut q, 821, 1000), 1);
    // Its turn: the merge gate fails on the approved head.
    let gate = format!("{GATE}test");
    let started = c.apply(Event::Merge { by: WINDOW.into() }).expect("merge");
    assert_eq!(started, [Effect::Start(gate.clone())]);
    c.apply(Event::Started {
        job: gate.clone(),
        sha: E.into(),
    })
    .expect("gate");
    let before = c.clone();
    c.apply(Event::Finished {
        job: gate,
        sha: E.into(),
        ok: false,
    })
    .expect("the gate fails");
    assert!(control::leaves_merge_queue(Some(&before), &c));
    assert_eq!(merges::failed_at(&c), Some(2));
    let (q, next) = merges::left(&q, 821, merges::failed_at(&c), 1100);
    assert_eq!(next.map(|e| e.number), Some(1002), "the next starts");
    assert!(q.returning.is_empty(), "no second kept place");
    c.apply(Event::Pushed { sha: F.into() })
        .expect("a second fix");
    approved_with_own(&mut c, F, "fixed again", 1200);
    assert_eq!(c.pushes, 3);
    let (_, queued) =
        merges::enqueue_back(&q, 821, WINDOW, &Back::of(&c), 1300).expect("asked again");
    assert_eq!(queued, Queued::Behind(15), "at the back");
}

#[test]
fn a_rebase_pushed_onto_the_holder_with_its_own_lines_takes_the_turn_back_first() {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(1000, WINDOW, 0), Ok(Queued::Start));
    let asked = Back {
        own: Some("o1".into()),
        pushes: 1,
    };
    let (mut q, _) = merges::enqueue_back(&q, 674, WINDOW, &asked, 0).expect("queued");
    q.enqueue(2000, WINDOW, 0).expect("behind");
    q.enqueue(2001, WINDOW, 0).expect("behind");
    until_turn(&mut q, 674, 100);
    let mut c = parked_holder(674);
    c.apply(Event::Pushed { sha: C.into() })
        .expect("a rebase onto main");
    let before = c.clone();
    bound_own(&mut c, C, "p2", "o1");
    assert_eq!(c.merge_after, None, "its approval didn't carry");
    assert!(control::leaves_merge_queue(Some(&before), &c));
    assert_eq!(merges::failed_at(&c), None);
    let (left, next) = merges::left(&q, 674, None, 200);
    assert_eq!(next.map(|e| e.number), Some(2000), "the next starts");
    c.apply(Event::Approve {
        by: WINDOW.into(),
        sha: C.into(),
        at: 300,
    })
    .expect("approved again");
    assert_eq!(c.own.as_deref(), Some("o1"));
    let (q, queued) =
        merges::enqueue_back(&left, 674, WINDOW, &Back::of(&c), 400).expect("asked again");
    assert_eq!(queued, Queued::Behind(1), "first in line");
    assert_eq!(place(&q, 674), Some((2, 3)));
    let changed = Back {
        own: Some("o2".into()),
        pushes: 2,
    };
    let (_, queued) = merges::enqueue_back(&left, 674, WINDOW, &changed, 400).expect("asked again");
    assert_eq!(
        queued,
        Queued::Behind(2),
        "its own lines changed: at the back"
    );
}

#[test]
fn a_session_push_is_counted_and_its_own_lines_kept_but_a_restack_is_neither() {
    let mut c = approved(10);
    assert_eq!(c.pushes, 1);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, false))
        .expect("restacked");
    assert_eq!(
        (c.head.as_deref(), c.pushes, c.own.as_deref()),
        (Some(C), 1, None)
    );
    c.apply(Event::Pushed { sha: E.into() }).expect("pushed");
    bound_own(&mut c, E, "p", "o");
    assert_eq!((c.pushes, c.own.as_deref()), (2, Some("o")));
    c.apply(Event::Pushed { sha: E.into() })
        .expect("the same head again");
    assert_eq!(c.pushes, 2);
}

/// Change `n` takes its turn while behind, and its clean restack keeps the patch, so its approval
/// carries and its merge parks on its new head's checks.
fn parked_on_restack(n: u32, same_patch: bool) -> Change {
    let mut c = approved(n);
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn");
    c.apply(restacked(Relation::Clean, same_patch))
        .expect("restacked");
    assert!(c.merge_after.is_some(), "its merge is parked");
    c
}

#[test]
fn replay_804_a_holder_whose_ios_check_waits_for_a_machine_yields_and_takes_its_turn_back() {
    let mut q = queue(804, 0, 1);
    until_turn(&mut q, 804, 100);
    let c = parked_on_restack(804, true);
    assert!(merges::machine_yieldable(&c));
    let before = c.clone();
    let c = c
        .yield_to(
            merges::MACHINE,
            "pigeon",
            "its `ios` check waits for a macOS machine",
            150,
        )
        .expect("yields");
    assert!(merges::yielded_for_machine(&c));
    assert!(!merges::machine_yieldable(&c));
    assert!(control::leaves_merge_queue(Some(&before), &c));
    assert!(merges::keeps_place(&c));
    assert_eq!(
        control::release_reason(&c),
        "its `ios` check waits for a macOS machine; it keeps its place"
    );
    q.yielded.insert(804, merges::MACHINE);
    let (mut q, next) = merges::yielded_back(&q, 804, 160);
    assert_eq!(next.map(|e| e.number), Some(2000), "the next starts");
    assert_eq!(place(&q, 804), Some((2, 2)), "first in line");
    let yielded = c.clone();
    let mut c = c.unyield();
    assert!(control::takes_turn_back(Some(&yielded), &c));
    assert!(!control::takes_turn_back(Some(&c), &c));
    q.yielded.clear();
    assert_eq!(
        q.done(2000, 300).map(|e| e.number),
        Some(804),
        "it takes the turn back in its place"
    );
    c.apply(Event::Merge { by: WINDOW.into() })
        .expect("its turn again");
    assert!(
        c.merge_after.is_some(),
        "its merge parks on its checks again"
    );
}

#[test]
fn a_holder_whose_approval_does_not_carry_keeps_its_turn_while_its_ios_check_waits() {
    let c = parked_on_restack(804, false);
    assert!(
        !merges::machine_yieldable(&c),
        "taking the turn back could not park its merge"
    );
}

#[test]
fn a_change_yielded_to_another_is_not_taken_back_for_a_machine() {
    let c = approved(9).yield_to(673, "agent", "r", 1).expect("yields");
    assert!(!merges::yielded_for_machine(&c));
    assert!(!control::takes_turn_back(Some(&c), &c.clone().unyield()));
}
