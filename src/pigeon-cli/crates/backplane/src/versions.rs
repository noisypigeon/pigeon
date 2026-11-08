//! Which of two Worker versions is newer, so a Durable Object running older code than its caller
//! resets itself; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md`, the amendment on
//! busy Durable Objects.

/// The header a caller names its version in: its timestamp, a space, and its ID.
pub const HEADER: &str = "x-pigeon-version";

/// The header a stale object's answer carries, so its caller retries.
pub const STALE: &str = "x-pigeon-stale";

/// A version as the header carries it.
#[must_use]
pub fn stamp(timestamp: &str, id: &str) -> String {
    format!("{timestamp} {id}")
}

/// Whether a caller stamped `caller` runs newer code than an object stamped `mine`: both known,
/// different, and the caller's timestamp later. Timestamps are RFC 3339 in UTC, so they compare
/// as text.
#[must_use]
pub fn caller_is_newer(caller: Option<&str>, mine: &str) -> bool {
    let Some(caller) = caller.map(str::trim).filter(|c| !c.is_empty()) else {
        return false;
    };
    let (Some((theirs, their_id)), Some((ours, our_id))) =
        (caller.split_once(' '), mine.trim().split_once(' '))
    else {
        return false;
    };
    their_id != our_id && !ours.is_empty() && theirs > ours
}

/// Whether an object answering `answered` to `/version` is current for a Worker stamped `mine`:
/// it answered, and isn't older. An empty answer is an object without the handshake, which
/// can't reset itself and is replaced when it idles; it counts as current.
#[must_use]
pub fn object_current(answered: Option<&str>, mine: &str) -> bool {
    answered.is_some_and(|v| v.trim().is_empty() || !caller_is_newer(Some(mine), v))
}
