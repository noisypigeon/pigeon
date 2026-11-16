//! The owner's CI window; see
//! `.claude/decisions/2026-11-10-development-code-checks-github-actions.md`, its 2026-11-11
//! amendment.

use backplane::ci_window::{self, CiWindow, Window};

const HOUR: u64 = 3600;

#[test]
fn a_window_is_open_until_its_end_and_closed_after() {
    let w = Window {
        until: 100,
        by: "owner".into(),
    };
    assert!(w.open(99));
    assert!(!w.open(100), "expired: closed without anyone closing it");
    let all = CiWindow {
        window: Some(w),
        ..CiWindow::default()
    };
    assert!(all.current(50).is_some());
    assert!(all.current(150).is_none());
    assert!(CiWindow::default().current(0).is_none());
}

#[test]
fn granting_an_offer_opens_replaces_or_closes_the_window() {
    let mut all = CiWindow::default();
    let six = all.offer(6, "t6", 0, 10 * HOUR);
    assert_eq!((six.id, six.hours), (1, 6));
    assert!(all.current(HOUR).is_none(), "an offer opens nothing");
    assert_eq!(all.peek("t6", HOUR).map(|o| o.id), Ok(1));
    let w = all
        .grant("t6", "owner", HOUR)
        .expect("granted")
        .expect("open");
    assert_eq!(w.until, 7 * HOUR);
    assert!(all.grant("t6", "owner", HOUR).is_err(), "spent");
    assert!(all.current(6 * HOUR).is_some());
    assert!(all.current(7 * HOUR).is_none());

    all.offer(2, "t2", HOUR, 10 * HOUR);
    all.grant("t2", "owner", 2 * HOUR).expect("granted");
    assert_eq!(
        all.current(2 * HOUR).map(|w| w.until),
        Some(4 * HOUR),
        "replaced"
    );

    all.offer(0, "t0", 2 * HOUR, 10 * HOUR);
    assert_eq!(all.grant("t0", "owner", 3 * HOUR), Ok(None));
    assert!(all.current(3 * HOUR).is_none(), "closed early");
}

#[test]
fn an_expired_or_unknown_link_grants_nothing() {
    let mut all = CiWindow::default();
    all.offer(6, "t", 0, 100);
    assert!(all.peek("t", 100).is_err());
    assert!(all.grant("t", "owner", 100).is_err());
    assert!(all.peek("other", 0).is_err());
    assert!(all.current(100).is_none());
}

#[test]
fn hours_are_zero_to_twenty_four() {
    assert_eq!(ci_window::hours(Some("6")), Ok(6));
    assert_eq!(ci_window::hours(Some(" 0 ")), Ok(0));
    assert_eq!(ci_window::hours(Some("24")), Ok(24));
    assert!(ci_window::hours(Some("25")).is_err());
    assert!(ci_window::hours(Some("-1")).is_err());
    assert!(ci_window::hours(None).is_err());
}
