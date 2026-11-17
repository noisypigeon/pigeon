//! A head that fails, or a holder that yields, gives its turn back; see the 2026-11-16 amendment
//! to `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.

use crate::harness::step;

use agent::control;
use agent::core::approval::tiers::{Tier, Tiered, Tiers};
use agent::core::change::queue::{self as merges, MergeQueue, Queued};
use agent::core::change::stage::{self, Needs};
use agent::core::change::{
    Change, Check, Effect, Event, GATE, MERGE, RESTACK, Relation, Restacked, State,
};
use step::{Apply, Queue};
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
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
