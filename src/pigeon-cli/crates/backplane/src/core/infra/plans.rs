//! Stored plans and completed applies: their records and checks, given the time; see the
//! phase 3 amendment §5 of `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use serde::{Deserialize, Serialize};

use crate::cobot::Change;

/// The directory under the state holding stored plans.
pub const DIR: &str = "plans";
/// Each plan's manifest.
pub const MANIFEST: &str = "plan.json";
/// Each unit's saved plan file, encrypted like state.
pub const FILE: &str = "plan.tfplan";
/// How long a stored plan may be applied: 24 hours.
pub const LIFETIME_SECS: u64 = 24 * 60 * 60;
/// The directory under [`DIR`] holding completed plans' records.
pub const COMPLETED: &str = "completed";
/// How long a completed plan's record is kept: 7 days.
pub const COMPLETED_SECS: u64 = 7 * 24 * 60 * 60;

/// A completed apply: what each unit applied, kept after its plan is removed so a retry can
/// report it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Completed {
    /// The plan's ID.
    pub id: String,
    /// When the apply completed, in Unix seconds.
    pub completed: u64,
    /// Each unit's changes and note, in apply order.
    pub units: Vec<CompletedUnit>,
}

/// One unit of a completed apply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompletedUnit {
    /// The unit's path.
    pub unit: String,
    /// Its changes: addresses, actions and attribute names only.
    pub changes: Vec<Change>,
    /// Its note, if any.
    pub note: Option<String>,
}

/// A stored plan's manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    /// Its ID: hex, from its units, plans and creation time.
    pub id: String,
    /// When it was stored, in Unix seconds.
    pub created: u64,
    /// After this, in Unix seconds, it is refused.
    pub expires: u64,
    /// Its units, in apply order.
    pub units: Vec<StoredUnit>,
}

/// One unit of a stored plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredUnit {
    /// The unit's path.
    pub unit: String,
    /// The SHA-256 of its desired state when planned.
    pub digest: String,
    /// Its planned changes: addresses and actions only.
    pub changes: Vec<Change>,
}

/// Whether `id` is a plan ID: 32 lowercase hex digits.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Plan `id` from its manifest's `text` (none when there is no stored plan), refused when
/// malformed, unknown or expired at `now`.
///
/// # Errors
///
/// Why it is refused.
pub fn check(id: &str, text: Option<&str>, now: u64) -> Result<Stored, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a plan ID"));
    }
    let text = text.ok_or_else(|| format!("no stored plan {id}"))?;
    let stored: Stored =
        serde_json::from_str(text).map_err(|e| format!("plan {id}'s manifest: {e}"))?;
    if stored.id != id || stored.units.is_empty() {
        return Err(format!("plan {id}'s manifest does not match it"));
    }
    if now >= stored.expires {
        return Err(format!("plan {id} expired; plan again"));
    }
    Ok(stored)
}

/// The record of plan `id`'s completed apply in `text`, if it completed within
/// [`COMPLETED_SECS`] of `now`.
#[must_use]
pub fn completed_of(id: &str, text: &str, now: u64) -> Option<Completed> {
    let record: Completed = serde_json::from_str(text).ok()?;
    (record.id == id && now < record.completed + COMPLETED_SECS).then_some(record)
}

/// Whether a completed record's `text` is unreadable or older than [`COMPLETED_SECS`] at `now`.
#[must_use]
pub fn expired(text: Option<&str>, now: u64) -> bool {
    text.and_then(|t| serde_json::from_str::<Completed>(t).ok())
        .is_none_or(|r| now >= r.completed + COMPLETED_SECS)
}
