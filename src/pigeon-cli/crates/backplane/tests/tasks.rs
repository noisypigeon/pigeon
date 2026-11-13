//! Tasks and session limits; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5, §6.

use backplane::sessions::{MAX_OPEN, MAX_PER_SESSION, Open, family, limit};
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
        slug: String::new(),
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

fn generated(number: u32, slug: &str, state: &str) -> Open {
    Open {
        slug: slug.into(),
        state: state.into(),
        ..open(number, Some("s-a"))
    }
}

#[test]
fn a_change_and_its_unpushed_generated_changes_hold_one_slot() {
    assert_eq!(family("reconcile-79"), Some(79));
    assert_eq!(family("review-61-59"), Some(61));
    assert_eq!(family("reconcile-x"), None);
    assert_eq!(family("feature-79"), None);
    // !79 and four others fill four slots; ten unpushed reconciliations and reviews of !79 add none.
    let mut open_now: Vec<Open> = (1..=3).map(|n| open(n, Some("s-a"))).collect();
    open_now.push(open(79, Some("s-a")));
    for n in 85..90 {
        open_now.push(generated(n, "reconcile-79", "proposed"));
    }
    for n in 100..105 {
        open_now.push(generated(n, &format!("review-79-{n}"), "planned"));
    }
    assert_eq!(limit(&open_now, Some("s-a")), Ok(()), "four slots held");
    // Once one is pushed it is real work, and the fifth slot is taken.
    open_now.push(generated(110, "reconcile-79", "pushed"));
    let why = limit(&open_now, Some("s-a")).expect_err("full");
    assert!(why.contains("!79 (+!85 +!86"), "{why}");
    assert!(why.contains("!110"), "{why}");
    let many: Vec<Open> = (1..=40)
        .map(|n| generated(n, &format!("reconcile-{}", n + 100), "proposed"))
        .collect();
    assert_eq!(
        limit(&many, None),
        Ok(()),
        "the instance counts none of them"
    );
}
