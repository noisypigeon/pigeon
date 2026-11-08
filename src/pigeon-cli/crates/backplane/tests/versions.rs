//! A Durable Object older than its caller resets itself; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md`, the amendment on
//! busy Durable Objects.

use backplane::versions::{caller_is_newer, stamp};

#[test]
fn only_a_later_caller_with_another_id_makes_an_object_stale() {
    let old = stamp("2026-11-08T13:00:00.000Z", "aaaa");
    let new = stamp("2026-11-08T14:00:00.000Z", "bbbb");
    assert!(caller_is_newer(Some(&new), &old));
    assert!(
        !caller_is_newer(Some(&old), &new),
        "an older caller never resets a newer object"
    );
    assert!(!caller_is_newer(Some(&new), &new), "the same version");
    assert!(
        !caller_is_newer(Some(&stamp("2026-11-08T14:00:00.000Z", "aaaa")), &old),
        "the same id isn't another version"
    );
    assert!(!caller_is_newer(None, &old), "a caller naming no version");
    assert!(!caller_is_newer(Some(""), &old));
    assert!(
        !caller_is_newer(Some(&new), ""),
        "an object that doesn't know its version"
    );
}

#[test]
fn an_object_without_the_handshake_counts_as_current_and_a_silent_one_does_not() {
    use backplane::versions::object_current;
    let mine = stamp("2026-11-08T21:00:00Z", "new");
    assert!(object_current(Some(""), &mine));
    assert!(object_current(Some(&mine), &mine));
    assert!(object_current(
        Some(&stamp("2026-11-08T22:00:00Z", "newer")),
        &mine
    ));
    assert!(!object_current(
        Some(&stamp("2026-11-08T20:00:00Z", "old")),
        &mine
    ));
    assert!(!object_current(None, &mine));
}
