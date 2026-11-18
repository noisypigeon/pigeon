//! Rows of the D1 `journal` table, which objects append to in shadow; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §A.5. Not `ChangeRun`'s DO-local `journal` key.

use serde_json::json;

/// The epoch every shadow entry is written at.
pub const SHADOW_EPOCH: u64 = 0;

/// An entry for `object` at `seq`; a second append of the same key is a no-op.
#[must_use]
pub fn entry(
    object: &str,
    seq: u64,
    epoch: u64,
    at: u64,
    kind: &str,
    body: &serde_json::Value,
) -> crate::control::Statement {
    crate::control::Statement {
        sql: "INSERT OR IGNORE INTO journal (object, seq, epoch, at, kind, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params: vec![
            json!(object),
            json!(seq),
            json!(epoch),
            json!(at),
            json!(kind),
            json!(body.to_string()),
        ],
    }
}

/// The entry for an Instance's event-log event.
#[must_use]
pub fn event_entry(
    instance: &str,
    e: &crate::events::Event,
    epoch: u64,
) -> crate::control::Statement {
    entry(
        &format!("instance:{instance}/events"),
        e.id,
        epoch,
        e.at,
        "event",
        &serde_json::to_value(e).unwrap_or_default(),
    )
}

/// The Instance keys journaled as snapshots; see the ADR's 2026-11-18 amendment on identity state.
pub const INSTANCE_KEYS: &[&str] = &[
    "accounts",
    "members",
    "passkeys",
    "passkey-adding",
    "passkey-enrolments",
    "passkey-proofs",
    "totp",
    "standalone.seeded",
    "instances.private",
    "tickets",
    "yields",
    "pins",
    "queue",
    "tasks",
    "briefs",
    "mandates",
    "windows",
    "lanes",
    "owner_items",
    "event-cursors",
    "push-latest",
    "digest-cursor",
    "grant-registry",
    "ci-window",
    "macos-health",
    "merges",
    "bench-fleet",
    "bench-used",
    "learner.index",
];

/// Instance keys holding secret material, never journaled until owner item #42 is answered.
pub const WITHHELD: &[&str] = &[
    "devices",
    "waiting",
    "grant-requests",
    "apns_token",
    "push-devices",
    "members.joins",
];

/// The `MainRun` keys journaled as snapshots; see the ADR's 2026-11-18 amendment on identity state.
pub const MAIN_KEYS: &[&str] = &[
    "pipeline",
    "mirroring",
    "ledger",
    "releases",
    "evictions",
    "test_maps",
    "ios_releases",
    "starting",
    "platform-runs",
];

/// `MainRun` keys that may hold infra outputs, withheld with owner item #42's.
pub const MAIN_WITHHELD: &[&str] = &["platform-outputs"];

/// The most JSON text one snapshot entry holds; a larger value isn't journaled until it shrinks.
pub const MAX_VALUE: usize = 1_000_000;

/// The DO-local key holding each journaled key's last digest.
pub const DIGESTS: &str = "journal.digests";

/// The DO-local key holding the snapshot object's last sequence number.
pub const SEQ: &str = "journal.seq";

/// The keys among `now` whose value changed since `previous`, each with its new digest (SHA-256 hex of the value's JSON text); a key in `previous` missing from `now` changes to an empty digest; a value whose JSON text exceeds [`MAX_VALUE`] is left out, keeping its previous digest.
#[must_use]
pub fn changed(
    previous: &std::collections::BTreeMap<String, String>,
    now: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<(String, String)> {
    use sha2::Digest as _;
    let mut out = Vec::new();
    for (k, v) in now {
        let text = v.to_string();
        if text.len() > MAX_VALUE {
            continue;
        }
        let d = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
        if previous.get(k) != Some(&d) {
            out.push((k.clone(), d));
        }
    }
    for (k, d) in previous {
        if !now.contains_key(k) && !d.is_empty() {
            out.push((k.clone(), String::new()));
        }
    }
    out
}

/// The last sequence number `object` holds in D1, 0 for none.
#[must_use]
pub fn last_seq(object: &str) -> crate::control::Statement {
    crate::control::Statement {
        sql: "SELECT COALESCE(MAX(seq), 0) AS seq FROM journal WHERE object = ?1",
        params: vec![json!(object)],
    }
}

/// The Instance's state object.
#[must_use]
pub fn state_object(instance: &str) -> String {
    format!("instance:{instance}/state")
}

/// The `MainRun`'s state object.
#[must_use]
pub fn main_object(instance: &str) -> String {
    format!("main:{instance}/state")
}

/// A `ChangeRun`'s state object.
#[must_use]
pub fn change_object(instance: &str, number: u64) -> String {
    format!("change:{instance}/{number}/state")
}

/// A snapshot entry for the Instance's `key`, at `seq` of its state object.
#[must_use]
pub fn snapshot_entry(
    instance: &str,
    seq: u64,
    at: u64,
    key: &str,
    value: &serde_json::Value,
) -> crate::control::Statement {
    snapshot_entry_at(instance, seq, SHADOW_EPOCH, at, key, value)
}

/// A snapshot entry at `epoch`.
#[must_use]
pub fn snapshot_entry_at(
    instance: &str,
    seq: u64,
    epoch: u64,
    at: u64,
    key: &str,
    value: &serde_json::Value,
) -> crate::control::Statement {
    entry(&state_object(instance), seq, epoch, at, key, value)
}
