//! Stored plans and completed applies: their records and checks, given the time; see the
//! phase 3 amendment §5 of `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use serde::{Deserialize, Serialize};

use super::engines::{Action, Change};

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
    /// Whether it was planned with `--adopt`; its apply then refuses to destroy a DNS record (see
    /// .claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md §5).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adopted: bool,
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

/// Why plan `stored` may not be applied: it adopted live resources and would replace or delete a
/// DNS record.
#[must_use]
pub fn adoption_refused(stored: &Stored) -> Option<String> {
    if !stored.adopted {
        return None;
    }
    let gone: Vec<String> = stored
        .units
        .iter()
        .flat_map(|u| &u.changes)
        .filter(|c| {
            matches!(c.action, Action::Replace | Action::Delete)
                && c.address.contains(".cloudflare_dns_record.")
        })
        .map(|c| format!("{} ({})", c.address, c.action.word()))
        .collect();
    if gone.is_empty() {
        return None;
    }
    Some(format!(
        "plan {} adopted live resources and would destroy DNS records: {}; an adopted zone's first apply keeps every record, so declare them as they are live and plan again",
        stored.id,
        gone.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = "module.dns_records[\"example.com:www-a\"].cloudflare_dns_record.this";

    fn stored(adopted: bool, address: &str, action: Action) -> Stored {
        Stored {
            id: "0".repeat(32),
            created: 0,
            expires: 1,
            units: vec![StoredUnit {
                unit: "zones/example.com".into(),
                digest: String::new(),
                changes: vec![Change {
                    address: address.into(),
                    action,
                    attributes: vec![],
                }],
            }],
            adopted,
        }
    }

    #[test]
    fn an_adopting_plan_that_destroys_a_dns_record_is_refused() {
        let why = adoption_refused(&stored(true, RECORD, Action::Replace)).expect("refused");
        assert!(why.contains("would destroy DNS records"), "{why}");
        assert!(why.contains("(replace)"), "{why}");
        let why = adoption_refused(&stored(true, RECORD, Action::Delete)).expect("refused");
        assert!(why.contains("(delete)"), "{why}");
    }

    #[test]
    fn creates_updates_and_plans_without_adopt_pass() {
        assert!(adoption_refused(&stored(true, RECORD, Action::Create)).is_none());
        assert!(adoption_refused(&stored(true, RECORD, Action::Update)).is_none());
        assert!(adoption_refused(&stored(false, RECORD, Action::Delete)).is_none());
        let bucket = "module.buckets[\"x\"].cloudflare_r2_bucket.this";
        assert!(adoption_refused(&stored(true, bucket, Action::Delete)).is_none());
    }

    #[test]
    fn a_manifest_without_adopted_reads_as_not_adopting() {
        let s = stored(false, RECORD, Action::Create);
        let json = serde_json::to_string(&s).expect("json");
        assert!(!json.contains("adopted"));
        let back = check(&s.id, Some(&json), 0).expect("check");
        assert!(!back.adopted);
    }
}
