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
    "zones.private",
];

/// Instance keys never journaled: each color mints its own APNs provider token; see the ADR's #42 amendment.
pub const WITHHELD: &[&str] = &["apns_token"];

/// The `MainRun` keys journaled as snapshots; see the ADR's 2026-11-18 amendment on identity state.
pub const MAIN_KEYS: &[&str] = &[
    "pipeline",
    "promote.hold",
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
    "zones.private",
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
    let mut out = Vec::new();
    for (k, v) in now {
        let text = v.to_string();
        if text.len() > MAX_VALUE {
            continue;
        }
        let d = digest(v);
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

/// A value's digest, as [`DIGESTS`] keeps it: SHA-256 hex of its JSON text.
#[must_use]
pub fn digest(value: &serde_json::Value) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(value.to_string().as_bytes()))
}

/// One journal row of a `*/state` object: its `seq`, its `kind` (the key) and its `body` (the value, or `{"sealed": …}` for a [`SEALED`] key).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its place in the object's journal.
    pub seq: u64,
    /// The key it snapshots.
    pub kind: String,
    /// The key's value as journaled.
    pub body: serde_json::Value,
}

/// The digest of each key's latest value in `rows`, as [`restore`] folds them.
#[must_use]
pub fn replay(
    rows: &[Row],
    open: &dyn Fn(&str, &serde_json::Value) -> Option<serde_json::Value>,
) -> (std::collections::BTreeMap<String, String>, Vec<String>) {
    let (values, unopened) = restore(rows, open);
    let map = values.iter().map(|(k, v)| (k.clone(), digest(v))).collect();
    (map, unopened)
}

/// Each key's latest value in `rows`, taken in `seq` order: a `null` body removes the key; a [`SEALED`] key's body is first opened by `open(key, body)`, and a key `open` can't open is left out and named in the second value.
#[must_use]
pub fn restore(
    rows: &[Row],
    open: &dyn Fn(&str, &serde_json::Value) -> Option<serde_json::Value>,
) -> (
    std::collections::BTreeMap<String, serde_json::Value>,
    Vec<String>,
) {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| r.seq);
    let mut map = std::collections::BTreeMap::new();
    let mut unopened: Vec<String> = Vec::new();
    for row in &sorted {
        if row.body.is_null() {
            map.remove(&row.kind);
        } else if SEALED.contains(&row.kind.as_str()) {
            match open(&row.kind, &row.body) {
                Some(v) if v.is_null() => {
                    map.remove(&row.kind);
                }
                Some(v) => {
                    map.insert(row.kind.clone(), v);
                }
                None => {
                    map.remove(&row.kind);
                    if !unopened.contains(&row.kind) {
                        unopened.push(row.kind.clone());
                    }
                }
            }
        } else {
            map.insert(row.kind.clone(), row.body.clone());
        }
    }
    unopened.retain(|k| !map.contains_key(k));
    (map, unopened)
}

/// Where a standby's replayed digests differ from the primary's held ones, as `<key>: replayed <r>, held <h>` (with `none` for a missing side), sorted by key; empty when they agree.
#[must_use]
pub fn parity(
    replayed: &std::collections::BTreeMap<String, String>,
    held: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let keys: std::collections::BTreeSet<&String> = replayed.keys().chain(held.keys()).collect();
    let mut out = Vec::new();
    for k in keys {
        let (r, h) = (replayed.get(k), held.get(k));
        if r != h {
            out.push(format!(
                "{k}: replayed {}, held {}",
                r.map_or("none", String::as_str),
                h.map_or("none", String::as_str)
            ));
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

/// Every journal row of `object`, oldest first.
#[must_use]
pub fn rows(object: &str) -> crate::control::Statement {
    crate::control::Statement {
        sql: "SELECT seq, kind, body FROM journal WHERE object = ?1 ORDER BY seq",
        params: vec![json!(object)],
    }
}

/// [`Row`]s from [`rows`]' D1 answer; a row whose `seq` isn't a number, or whose `body` text isn't JSON, is left out.
#[must_use]
pub fn from_rows(rows: &[serde_json::Value]) -> Vec<Row> {
    rows.iter()
        .filter_map(|row| {
            Some(Row {
                seq: row["seq"].as_u64()?,
                kind: row["kind"].as_str()?.to_string(),
                body: serde_json::from_str(row["body"].as_str()?).ok()?,
            })
        })
        .collect()
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

#[cfg(test)]
mod replay_tests {
    use super::*;
    use serde_json::json;

    fn row(seq: u64, kind: &str, body: serde_json::Value) -> Row {
        Row {
            seq,
            kind: kind.to_string(),
            body,
        }
    }

    #[test]
    fn rows_parse_from_d1_and_skip_what_isnt_a_row() {
        let d1 = [
            json!({"seq": 2, "kind": "a", "body": "1"}),
            json!({"seq": "x", "kind": "b", "body": "2"}),
            json!({"seq": 3, "kind": "c", "body": "not json"}),
        ];
        assert_eq!(from_rows(&d1), vec![row(2, "a", json!(1))]);
    }

    #[test]
    fn replay_takes_each_keys_latest_value_and_a_null_removes_it() {
        let rows = [
            row(2, "a", json!(2)),
            row(1, "a", json!(1)),
            row(3, "b", json!("x")),
            row(4, "b", json!(null)),
        ];
        let (map, unopened) = replay(&rows, &|_, _| None);
        let want = std::collections::BTreeMap::from([("a".to_string(), digest(&json!(2)))]);
        assert_eq!(map, want);
        assert_eq!(unopened, Vec::<String>::new());
    }

    #[test]
    fn a_sealed_key_is_digested_opened_and_named_when_it_cant_be() {
        let rows = [
            row(1, "devices", json!({"sealed": "s1"})),
            row(2, "pages", json!({"sealed": "s2"})),
        ];
        let open = |k: &str, _: &serde_json::Value| (k == "devices").then(|| json!([1]));
        let (map, unopened) = replay(&rows, &open);
        let want = std::collections::BTreeMap::from([("devices".to_string(), digest(&json!([1])))]);
        assert_eq!(map, want);
        assert_eq!(unopened, vec!["pages".to_string()]);
    }

    #[test]
    fn restore_keeps_each_keys_latest_value_opened() {
        let rows = [
            row(1, "a", json!(1)),
            row(3, "a", json!({"x": 2})),
            row(2, "b", json!(true)),
            row(4, "b", json!(null)),
            row(5, "devices", json!({"sealed": "s"})),
        ];
        let open = |_: &str, _: &serde_json::Value| Some(json!(["d"]));
        let (values, unopened) = restore(&rows, &open);
        let want = std::collections::BTreeMap::from([
            ("a".to_string(), json!({"x": 2})),
            ("devices".to_string(), json!(["d"])),
        ]);
        assert_eq!(values, want);
        assert_eq!(unopened, Vec::<String>::new());
    }

    #[test]
    fn parity_names_each_differing_key_by_digest_only() {
        let m = |p: &[(&str, &str)]| {
            p.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let replayed = m(&[("a", "1"), ("b", "2")]);
        let held = m(&[("a", "1"), ("c", "3")]);
        assert_eq!(
            parity(&replayed, &held),
            vec!["b: replayed 2, held none", "c: replayed none, held 3"]
        );
        assert_eq!(parity(&replayed, &replayed.clone()), Vec::<String>::new());
    }
}
