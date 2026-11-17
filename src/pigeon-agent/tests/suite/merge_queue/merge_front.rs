//! The owner moves a queued change to the front; see the 2026-11-15 amendment to
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.

use crate::harness::step;

use agent::core::change::queue::{self as merges, MergeQueue, Queued};
use step::Queue;

/// Change 1000 merging and `n` waiting behind it, 2001 to 2000 + `n`, as `queued 2/…` onwards.
fn queue(n: u32) -> MergeQueue {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(1000, "window:9", 0), Ok(Queued::Start));
    for i in 1..=n {
        q.enqueue(2000 + i, "window:9", 0).expect("queued");
    }
    q
}

fn order(q: &MergeQueue) -> Vec<u32> {
    q.waiting.iter().map(|e| e.number).collect()
}

#[test]
fn the_last_change_moves_to_the_front_behind_the_merge_running() {
    // !585 was 17/17.
    let q = queue(16);
    let (q, left) = merges::front(&q, 2016).expect("moved");
    assert_eq!(left, (17, 17));
    assert_eq!(
        q.merging.as_ref().map(|e| e.number),
        Some(1000),
        "not preempted"
    );
    assert_eq!(order(&q)[..3], [2016, 2001, 2002]);
    assert_eq!(q.waiting.len(), 16);
    let mut q = q;
    assert_eq!(q.done(1000, 10).map(|e| e.number), Some(2016));
}

#[test]
fn a_move_is_refused_when_merging_unqueued_or_yielded() {
    let mut q = queue(2);
    assert!(merges::front(&q, 1000).is_err(), "already merging");
    assert!(merges::front(&q, 7).is_err(), "not queued");
    q.yielded.insert(2002, 2001);
    let refused = merges::front(&q, 2002).expect_err("yielded");
    assert!(refused.contains("yields to !2001"), "{refused}");
}

#[test]
fn a_kept_change_still_goes_first() {
    let mut q = queue(3);
    // 2001 merges, 2002 loses its turn to main moving and is asked again.
    assert_eq!(q.done(1000, 1).map(|e| e.number), Some(2001));
    q.enqueue(2002, "window:9", 2).ok();
    let mut q = merges::keep(&q, 2002, 3);
    assert_eq!(q.release(2002, 4), None, "2001 still merges");
    assert_eq!(q.enqueue(2002, "window:9", 5), Ok(Queued::Behind(1)));
    let (q, _) = merges::front(&q, 2003).expect("moved");
    assert_eq!(order(&q), [2002, 2003]);
}
