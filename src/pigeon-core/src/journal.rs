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
    "devices",
    "waiting",
    "grant-requests",
    "push-devices",
    "members.joins",
];

/// Instance keys never journaled: each color mints its own APNs provider token; see the ADR's #42 amendment.
pub const WITHHELD: &[&str] = &["apns_token"];

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
    "platform-outputs",
];

/// The `ChangeRun` keys journaled as snapshots; `grants` is keyed by token hashes; see the ADR's #42 amendment.
pub const CHANGE_KEYS: &[&str] = &[
    "change",
    "pending",
    "journal",
    "logs",
    "failures",
    "breaker",
    "bound_log",
    "grants",
    "actions",
    "forecast",
    "after-seen",
    "macos",
    "queued",
    "pages",
];

/// Keys journaled sealed, as each object's own; see the ADR's #42 amendment.
pub const SEALED: &[&str] = &[
    "devices",
    "waiting",
    "grant-requests",
    "push-devices",
    "members.joins",
    "pages",
    "platform-outputs",
];

/// HKDF-SHA256 info for the journal's sealing key.
pub const SEAL_INFO: &[u8] = b"pigeon journal v1";

/// The journal's sealing key, from `journal-seal`'s 64 hex characters.
///
/// # Errors
/// The secret isn't 64 hex characters.
pub fn seal_key(secret: &str) -> Result<[u8; 32], String> {
    let t = secret.trim();
    if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("journal-seal is not 32 bytes of hex".into());
    }
    let ikm: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap_or_default())
        .collect();
    let mut okm = [0u8; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(None, &ikm)
        .expand(SEAL_INFO, &mut okm)
        .map_err(|e| e.to_string())?;
    Ok(okm)
}

/// A 12-byte nonce from 24 hex characters.
#[must_use]
pub fn nonce(hex: &str) -> Option<[u8; 12]> {
    if hex.len() != 24 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 12];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// `value` as `key`'s sealed body for `object`: `{"sealed": base64url(nonce‖ciphertext)}`, its associated data `<object>\n<key>`.
///
/// # Errors
/// When encryption fails.
pub fn sealed(
    seal: &[u8; 32],
    nonce: [u8; 12],
    object: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
    let aad = format!("{object}\n{key}");
    let msg = value.to_string();
    let ct = aes_gcm::Aes256Gcm::new(seal.into())
        .encrypt(
            aes_gcm::Nonce::from_slice(&nonce),
            Payload {
                msg: msg.as_bytes(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "sealing failed".to_string())?;
    let mut out = nonce.to_vec();
    out.extend(ct);
    Ok(json!({ "sealed": crate::mirroring::base64url(&out) }))
}

/// What [`sealed`] sealed, for `object` and `key`.
///
/// # Errors
/// Not a sealed body, or it doesn't open under `seal` as that object's key.
pub fn unseal(
    seal: &[u8; 32],
    object: &str,
    key: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
    let s = body
        .get("sealed")
        .and_then(serde_json::Value::as_str)
        .ok_or("not a sealed body")?;
    let raw = crate::access::base64url(s)?;
    if raw.len() < 12 + 16 {
        return Err("too short to be sealed".into());
    }
    let (n, ct) = raw.split_at(12);
    let aad = format!("{object}\n{key}");
    let plain = aes_gcm::Aes256Gcm::new(seal.into())
        .decrypt(
            aes_gcm::Nonce::from_slice(n),
            Payload {
                msg: ct,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "it doesn't open under this key".to_string())?;
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

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
