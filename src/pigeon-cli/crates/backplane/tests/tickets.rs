//! Session tickets; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §5.

use backplane::tickets::{TTL, Tickets, require, sign, verify};

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
