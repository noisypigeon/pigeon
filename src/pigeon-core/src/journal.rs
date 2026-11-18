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
