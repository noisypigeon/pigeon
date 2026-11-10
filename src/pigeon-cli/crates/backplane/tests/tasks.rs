//! Tasks and session limits; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5, §6.

use backplane::sessions::{MAX_OPEN, MAX_PER_SESSION, Open, limit};
use backplane::tasks::{KEPT, OWN, Tasks};

#[test]
fn a_task_is_its_sessions_first_then_anyones() {
    let mut t = Tasks::default();
    let id = t.add(
        "reconcile",
        70,
        Some(60),
        Some("s-a".into()),
        "fix: reconcile !60",
        100,
    );
    assert_eq!(id, 1);
    assert_eq!(
        t.add("reconcile", 70, Some(60), Some("s-a".into()), "again", 101),
        1,
        "one open task per kind and change"
    );
    assert_eq!(t.visible(Some("s-a"), 100).len(), 1);
    assert_eq!(t.visible(Some("s-b"), 100).len(), 0, "not yet");
    assert!(t.claim(1, "s-b", 100).is_err());
    assert_eq!(
        t.visible(Some("s-b"), 100 + OWN).len(),
        1,
        "after 30 minutes"
    );
    let taken = t.claim(1, "s-b", 100 + OWN).expect("claims");
    assert_eq!(taken.claimed_by.as_deref(), Some("s-b"));
    assert_eq!(t.visible(Some("s-a"), 100 + OWN).len(), 0, "taken");
    assert!(
        t.claim(1, "s-a", 100 + OWN)
            .expect_err("taken")
            .contains("s-b")
    );
    assert!(
        t.claim(1, "s-b", 100 + OWN).is_ok(),
        "its taker may claim again"
    );
    assert!(
        t.claim(1, " ", 100 + OWN).is_err(),
        "a claim names a session"
    );
}

#[test]
fn a_task_is_done_by_hand_or_when_its_change_merges() {
    let mut t = Tasks::default();
    t.add("reconcile", 70, None, None, "a", 0);
    t.add("review", 71, None, None, "b", 0);
    assert_eq!(
        t.visible(None, 0).len(),
        2,
        "a task for no session is anyone's"
    );
    assert_eq!(t.close_for(70), [1]);
    t.done(2).expect("done");
    assert_eq!(t.visible(None, 0).len(), 0);
    assert!(t.done(9).is_err());
    t.add("reconcile", 72, None, None, "c", KEPT);
    assert_eq!(t.list.len(), 1, "done tasks are kept a day");
}

fn open(number: u32, session: Option<&str>) -> Open {
    Open {
        number,
        state: "bounded".into(),
        session: session.map(str::to_string),
        scope: vec!["a/**".into()],
        affects: vec![],
        plan: String::new(),
    }
}

#[test]
fn a_session_holds_at_most_five_open_changes_and_the_instance_twenty() {
    let four: Vec<Open> = (1..=4).map(|n| open(n, Some("s-a"))).collect();
    assert_eq!(limit(&four, Some("s-a")), Ok(()));
    let five: Vec<Open> = (1..=5).map(|n| open(n, Some("s-a"))).collect();
    let why = limit(&five, Some("s-a")).expect_err("full");
    assert!(why.contains("!1, !2, !3, !4, !5"), "{why}");
    assert_eq!(MAX_PER_SESSION, 5);
    assert_eq!(limit(&five, Some("s-b")), Ok(()));
    assert_eq!(limit(&five, None), Ok(()));
    let nineteen: Vec<Open> = (1..=19).map(|n| open(n, None)).collect();
    assert_eq!(limit(&nineteen, None), Ok(()));
    let full: Vec<Open> = (1..=20).map(|n| open(n, None)).collect();
    assert_eq!(MAX_OPEN, 20);
    assert!(limit(&full, None).is_err());
}
