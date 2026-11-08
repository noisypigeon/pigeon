//! The merge queue; see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use backplane::changes::{Change, Event, State};
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
