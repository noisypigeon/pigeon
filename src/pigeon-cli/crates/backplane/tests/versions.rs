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
