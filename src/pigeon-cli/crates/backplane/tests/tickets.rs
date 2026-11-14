//! Session tickets; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §5.

use backplane::sessions::{Class, declare};
use backplane::tickets::{REMEMBER, TTL, Tickets, guarded, require, sign, verify};

const KEY: &[u8] = b"a session key for the tests only";
const A: &str = "https://claude.ai/code/session_a";

#[test]
fn a_ticket_verifies_until_it_expires_and_only_under_its_key() {
    let t = sign(KEY, A, 1, 100);
    assert_eq!(verify(KEY, &t, 99), Ok((A.to_string(), 1)));
    assert!(
        verify(KEY, &t, 100)
            .expect_err("expired")
            .contains("expired")
    );
    assert!(verify(b"another key", &t, 1).is_err());
    let forged = t.replacen(".1.", ".2.", 1);
    assert!(
        verify(KEY, &forged, 1).is_err(),
        "a changed generation breaks the signature"
    );
    assert!(verify(KEY, "nonsense", 1).is_err());
}

#[test]
fn a_label_holds_one_live_ticket_and_replacing_revokes_the_old() {
    let mut t = Tickets::default();
    let first = t.start(A, false, 0).expect("issues");
    assert_eq!((first.generation, first.expires), (1, TTL));
    assert!(t.start(A, false, 1).is_err(), "live already");
    let second = t.start(A, true, 1).expect("replaces");
    assert_eq!(second.generation, 2);
    assert_eq!(
        t.start(A, false, TTL + 2)
            .expect("expired, so anew")
            .generation,
        1
    );
}

#[test]
fn a_ticketed_label_needs_its_current_ticket_and_others_need_none() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    let a = |g| sign(KEY, A, g, TTL);
    let labels = vec![A.to_string()];
    assert_eq!(require(KEY, &t, &labels, Some(&a(1)), 5), Ok(()));
    assert!(
        require(KEY, &t, &labels, None, 5)
            .expect_err("missing")
            .contains("pass it as `ticket`")
    );
    let b = sign(KEY, "session_b", 1, TTL);
    assert!(
        require(KEY, &t, &labels, Some(&b), 5)
            .expect_err("another's")
            .contains("session_b's")
    );
    t.start(A, true, 6).expect("replaced");
    assert!(
        require(KEY, &t, &labels, Some(&a(1)), 7)
            .expect_err("old")
            .contains("replaced")
    );
    assert_eq!(
        require(KEY, &t, &["no-ticket-label".to_string()], None, 7),
        Ok(()),
        "a label without a ticket works as before"
    );
}

#[test]
fn ending_a_ticket_frees_its_label() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    assert!(t.end(A, 2));
    assert!(!t.end(A, 3), "a second end is a no-op");
    assert_eq!(require(KEY, &t, &[A.to_string()], None, 3), Ok(()));
    assert_eq!(t.start(A, false, 3).expect("anew").generation, 1);
}

#[test]
fn a_lapsed_ticket_is_remembered_a_week_to_say_how_long_its_session_has_gone() {
    let mut t = Tickets::default();
    let issued = t.start(A, false, 0).expect("starts");
    assert_eq!(t.gone_for(A, 10), None, "live");
    assert_eq!(t.gone_for(A, issued.expires + 60), Some(60));
    assert!(t.live(A, issued.expires + 60).is_none());
    // A lapsed ticket doesn't block a new start, which begins anew.
    let again = t.start(A, false, issued.expires + 60).expect("restarts");
    assert_eq!(again.generation, 1);
    // Past the week, another label's start forgets it.
    let later = again.expires + REMEMBER;
    t.start("https://claude.ai/code/session_b", false, later)
        .expect("starts");
    assert_eq!(t.gone_for(A, later), None, "forgotten");
}

#[test]
fn an_ended_ticket_is_remembered_from_its_end() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("starts");
    assert!(t.end(A, 100));
    assert!(t.live(A, 101).is_none(), "ended");
    assert_eq!(t.gone_for(A, 7300), Some(7200), "gone since its end");
    assert!(t.start(A, false, 7300).is_ok(), "and it may start again");
}

#[test]
fn a_ticket_keeps_its_class_and_a_replacement_may_declare_another() {
    let mut t = Tickets::default();
    assert_eq!(
        t.start(A, false, 0).expect("issues").declared.model,
        Class::Opus
    );
    let haiku = declare(Some("haiku"), Some("https://claude.ai/code/p"), Some("b3")).expect("ok");
    let second = t.start_as(A, true, 1, haiku).expect("replaces");
    assert_eq!(second.generation, 2);
    let live = t.live(A, 2).expect("live");
    assert_eq!(live.declared.model, Class::Haiku);
    assert_eq!(live.declared.brief.as_deref(), Some("b3"));
}

#[test]
fn a_label_s_class_outlives_its_ticket_and_an_unknown_label_has_none() {
    let mut t = Tickets::default();
    let sonnet = declare(Some("sonnet"), Some("https://claude.ai/code/p"), Some("b1")).expect("ok");
    t.start_as(A, false, 0, sonnet).expect("issues");
    assert!(t.end(A, 5));
    assert_eq!(t.class(A), Some(Class::Sonnet), "lapsed, still known");
    assert_eq!(t.class("https://claude.ai/code/unknown"), None);
}

#[test]
fn a_ticket_stored_before_classes_reads_as_opus() {
    let t: Tickets = serde_json::from_str(
        r#"{"https://claude.ai/code/session_a":{"generation":2,"expires":9}}"#,
    )
    .expect("an older record");
    assert_eq!(t.live(A, 1).expect("live").declared.model, Class::Opus);
}

#[test]
fn a_handoff_needs_the_change_holders_ticket_not_the_recipients() {
    const P: &str = "https://claude.ai/code/session_planner";
    assert_eq!(guarded("change.handoff", Some(P), Some(A)), vec![A]);
    assert_eq!(guarded("change.note", Some(P), Some(A)), vec![P, A]);
    assert_eq!(guarded("task.claim", Some(P), None), vec![P]);
}
