//! The release bot takes a merge turn; see .claude/decisions/2026-11-16-development-merge-path-measured-faster.md §1.

use crate::harness::step;

use agent::core::change::queue::{self as merges, MergeQueue, Queued};
use step::Queue;

/// Change 1000 merging and change 2001 waiting behind it.
fn busy() -> MergeQueue {
    let mut q = MergeQueue::default();
    assert_eq!(q.enqueue(1000, "window:9", 0), Ok(Queued::Start));
    assert_eq!(q.enqueue(2001, "window:9", 0), Ok(Queued::Behind(1)));
    q
}

#[test]
fn an_idle_queue_grants_the_turn_and_holds_merges_until_it_ends() {
    let (q, granted) = merges::release_turn(&MergeQueue::default(), 0, 100);
    assert!(granted);
    let mut q = q;
    assert_eq!(q.enqueue(772, "window:9", 110), Ok(Queued::Behind(1)));
    let (q, next) = merges::release_turn_done(&q, 150);
    assert_eq!(next.map(|e| e.number), Some(772));
    assert_eq!(q.merging.as_ref().map(|e| e.number), Some(772));
}

#[test]
fn a_busy_queue_refuses_a_fresh_target() {
    let (q, granted) = merges::release_turn(&busy(), 60, 100);
    assert!(!granted);
    assert_eq!(q.release_asked, None);
}

#[test]
fn a_target_waiting_30_minutes_reserves_the_next_turn() {
    let (mut q, granted) = merges::release_turn(&busy(), merges::RELEASE_WAIT, 100);
    assert!(!granted);
    assert_eq!(q.release_asked, Some(100));
    assert_eq!(q.done(1000, 110), None);
    assert_eq!(q.merging, None);
    let (q, granted) = merges::release_turn(&q, merges::RELEASE_WAIT, 120);
    assert!(granted);
    let (_, next) = merges::release_turn_done(&q, 160);
    assert_eq!(next.map(|e| e.number), Some(2001));
}

#[test]
fn a_reservation_not_taken_within_settle_lapses() {
    let (mut q, _) = merges::release_turn(&busy(), merges::RELEASE_WAIT, 100);
    assert_eq!(q.release_asked, Some(100));
    assert_eq!(
        q.done(1000, 100 + merges::SETTLE).map(|e| e.number),
        Some(2001)
    );
}

#[test]
fn a_turn_held_past_hold_lapses() {
    let (mut q, granted) = merges::release_turn(&MergeQueue::default(), 0, 0);
    assert!(granted);
    assert_eq!(q.enqueue(772, "window:9", merges::HOLD), Ok(Queued::Start));
}

#[test]
fn a_yielded_change_alone_leaves_the_queue_idle() {
    let mut q = MergeQueue::default();
    q.yielded.insert(2001, 1999);
    q.enqueue(2001, "window:9", 0).expect("queued");
    let (_, granted) = merges::release_turn(&q, 0, 100);
    assert!(granted);
}
