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
        &events_object(instance),
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

/// The most JSON text one journal row holds, under D1's 2 MB row; a longer body is written in
/// parts by [`entries`].
pub const MAX_VALUE: usize = 1_400_000;

/// The most JSON text one value journals, in however many parts; a larger value isn't journaled
/// until it shrinks, and [`oversized`] names it.
pub const MAX_TOTAL: usize = 16_000_000;

/// Each key among `now` whose JSON text exceeds [`MAX_TOTAL`], as `<key>: <bytes> bytes`, which
/// [`changed`] leaves out; see the blue-green ADR's 2026-11-20 oversized amendment.
#[must_use]
pub fn oversized(now: &std::collections::BTreeMap<String, serde_json::Value>) -> Option<String> {
    let over: Vec<String> = now
        .iter()
        .filter_map(|(k, v)| {
            let n = v.to_string().len();
            (n > MAX_TOTAL).then(|| format!("{k}: {n} bytes"))
        })
        .collect();
    (!over.is_empty()).then(|| format!("over the journal's {MAX_TOTAL} bytes: {}", over.join(", ")))
}

/// The DO-local key holding each journaled key's last digest.
pub const DIGESTS: &str = "journal.digests";

/// The tag of a map whose keys aren't all strings, as [`Stored`] keeps it: `{"$map": [[key, value], …]}`.
pub const MAP_TAG: &str = "$map";

/// A stored value as the journal keeps it: JSON, except that a map with a key that isn't a string
/// (a `BTreeMap<u32, _>`, which Workers storage holds as a JS `Map` with number keys) is kept
/// tagged by [`MAP_TAG`], so it journals and adopts back as it was. See
/// .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20
/// stored-shape amendment.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored(pub serde_json::Value);

impl<'de> serde::Deserialize<'de> for Stored {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StoredVisitor).map(Stored)
    }
}

/// [`Stored`]'s visitor: `serde_json`'s, but for a map's keys.
struct StoredVisitor;

impl<'de> serde::de::Visitor<'de> for StoredVisitor {
    type Value = serde_json::Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any stored value")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(v.into())
    }
    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
        Ok(serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, Into::into))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(v.into())
    }
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(v.into())
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::new();
        while let Some(Stored(v)) = a.next_element()? {
            out.push(v);
        }
        Ok(out.into())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut pairs = Vec::new();
        while let Some((Stored(k), Stored(v))) = a.next_entry()? {
            pairs.push((k, v));
        }
        if pairs.iter().all(|(k, _)| k.is_string()) {
            let map = pairs
                .into_iter()
                .map(|(k, v)| (k.as_str().unwrap_or_default().to_string(), v))
                .collect();
            return Ok(serde_json::Value::Object(map));
        }
        let pairs: Vec<serde_json::Value> = pairs.into_iter().map(|(k, v)| json!([k, v])).collect();
        Ok(json!({ MAP_TAG: pairs }))
    }
}

/// The `[key, value]` pairs of a value [`Stored`] tagged as a map with keys that aren't strings.
#[must_use]
pub fn tagged_map(v: &serde_json::Value) -> Option<Vec<(&serde_json::Value, &serde_json::Value)>> {
    let o = v.as_object().filter(|o| o.len() == 1)?;
    o.get(MAP_TAG)?
        .as_array()?
        .iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([k, v]) => Some((k, v)),
            _ => None,
        })
        .collect()
}

/// The DO-local key holding the snapshot object's last sequence number.
pub const SEQ: &str = "journal.seq";

/// The DO-local key holding the lease epoch an object last journaled under.
pub const EPOCH: &str = "journal.epoch";

/// The DO-local key holding why the object's last journal write failed; cleared by the next that lands.
pub const ERROR: &str = "journal.error";

/// `e` as [`ERROR`] keeps it: one line, at most 300 characters.
#[must_use]
pub fn error_text(e: &str) -> String {
    e.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

/// The digests [`changed`] compares against: `previous` while `journaled` is `epoch`; else each of `keys` as stale, so the first write under a new epoch journals every key afresh, and removes each it lacks, over whatever another colour journaled. See .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20 rebaseline amendment.
#[must_use]
pub fn baseline(
    previous: std::collections::BTreeMap<String, String>,
    journaled: Option<u64>,
    epoch: u64,
    keys: &[&str],
) -> std::collections::BTreeMap<String, String> {
    if journaled == Some(epoch) {
        return previous;
    }
    keys.iter()
        .map(|k| ((*k).to_string(), "stale".to_string()))
        .collect()
}

/// The keys among `now` whose value changed since `previous`, each with its new digest (SHA-256 hex of the value's JSON text); a key in `previous` missing from `now` changes to an empty digest; a value whose JSON text exceeds [`MAX_TOTAL`] is left out, keeping its previous digest.
#[must_use]
pub fn changed(
    previous: &std::collections::BTreeMap<String, String>,
    now: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (k, v) in now {
        let text = v.to_string();
        if text.len() > MAX_TOTAL {
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

/// The tag of one part of a body [`entries`] split: `{"$part": i, "of": n, "text": …}`.
pub const PART_TAG: &str = "$part";

/// `rows` in `seq` order with each run of parts joined into the one row it was split from, at
/// the last part's `seq`; a key whose parts don't make a whole body is left out and named in the
/// second value. See the blue-green ADR's 2026-11-20 parts amendment.
fn assembled(rows: Vec<Row>) -> (Vec<Row>, Vec<String>) {
    let mut out = Vec::new();
    let mut broken: Vec<String> = Vec::new();
    let mut open: std::collections::BTreeMap<String, (u64, Vec<String>)> =
        std::collections::BTreeMap::new();
    for row in rows {
        let Some(i) = row.body.get(PART_TAG).and_then(serde_json::Value::as_u64) else {
            open.remove(&row.kind);
            out.push(row);
            continue;
        };
        let of = row.body["of"].as_u64().unwrap_or(0);
        let text = row.body["text"].as_str().unwrap_or_default().to_string();
        let parts = open.entry(row.kind.clone()).or_insert((of, Vec::new()));
        if i == 0 {
            *parts = (of, Vec::new());
        }
        if parts.0 != of || parts.1.len() as u64 != i {
            open.remove(&row.kind);
            if !broken.contains(&row.kind) {
                broken.push(row.kind.clone());
            }
            continue;
        }
        parts.1.push(text);
        if i + 1 == of {
            let whole = parts.1.concat();
            open.remove(&row.kind);
            match serde_json::from_str(&whole) {
                Ok(body) => {
                    broken.retain(|k| k != &row.kind);
                    out.push(Row {
                        seq: row.seq,
                        kind: row.kind,
                        body,
                    });
                }
                Err(_) => broken.push(row.kind),
            }
        }
    }
    broken.extend(
        open.into_keys()
            .filter(|k| !broken.contains(k))
            .collect::<Vec<_>>(),
    );
    (out, broken)
}

/// What `c` takes in a JSON string once escaped.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// Room a part's row keeps for its tag, counters and quotes around the escaped text.
const PART_OVERHEAD: usize = 64;

/// The rows that journal `body` for `kind` from `seq` on: one [`entry`] while its JSON text fits
/// [`MAX_VALUE`], else that text in parts whose row text, with the chunk escaped again inside it,
/// stays within [`MAX_VALUE`], each tagged by [`PART_TAG`] at the next `seq`, written in the same
/// batch. See the blue-green ADR's 2026-11-20 parts amendment.
#[must_use]
pub fn entries(
    object: &str,
    seq: u64,
    epoch: u64,
    at: u64,
    kind: &str,
    body: &serde_json::Value,
) -> Vec<crate::control::Statement> {
    let text = body.to_string();
    if text.len() <= MAX_VALUE {
        return vec![entry(object, seq, epoch, at, kind, body)];
    }
    let mut chunks: Vec<&str> = Vec::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let mut room = MAX_VALUE - PART_OVERHEAD;
        let mut cut = rest.len();
        for (i, c) in rest.char_indices() {
            let n = escaped_len(c);
            if n > room {
                cut = i;
                break;
            }
            room -= n;
        }
        let (head, tail) = rest.split_at(cut);
        chunks.push(head);
        rest = tail;
    }
    let of = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let part = json!({ PART_TAG: i, "of": of, "text": t });
            entry(object, seq + i as u64, epoch, at, kind, &part)
        })
        .collect()
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
    let (sorted, mut unopened) = assembled(sorted);
    let mut map = std::collections::BTreeMap::new();
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

/// The journal rows of `object` that make each key's current version, oldest first: per key, its
/// rows from its latest whole row or first part (`{"$part":0,`, as `serde_json` writes the tag
/// first) on, so a read is bounded by the state, not the history; see the blue-green ADR's
/// 2026-11-20 current-version amendment.
#[must_use]
pub fn rows(object: &str) -> crate::control::Statement {
    crate::control::Statement {
        sql: "WITH starts AS (SELECT kind, MAX(seq) AS s FROM journal WHERE object = ?1 AND (body NOT LIKE '{\"$part\":%' OR body LIKE '{\"$part\":0,%') GROUP BY kind) SELECT j.seq, j.kind, j.body FROM journal j JOIN starts ON j.kind = starts.kind WHERE j.object = ?1 AND j.seq >= starts.s ORDER BY j.seq",
        params: vec![json!(object)],
    }
}

/// Deletes the journal rows of `object` that a later version of the same key supersedes: per key,
/// every row before the start [`rows`] reads from, so nothing a read or an adopt uses goes. Run in
/// the batch of each append, after it. The owner approved it (`owner_ask` #52); see the blue-green
/// ADR's 2026-11-20 compaction amendment.
#[must_use]
pub fn compact(object: &str) -> crate::control::Statement {
    crate::control::Statement {
        sql: "DELETE FROM journal WHERE object = ?1 AND seq < (SELECT MAX(k.seq) FROM journal k WHERE k.object = journal.object AND k.kind = journal.kind AND (k.body NOT LIKE '{\"$part\":%' OR k.body LIKE '{\"$part\":0,%'))",
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

/// The newest `n` journal rows of `object`, newest first.
#[must_use]
pub fn recent_rows(object: &str, n: u64) -> crate::control::Statement {
    crate::control::Statement {
        sql: "SELECT seq, kind, body FROM journal WHERE object = ?1 ORDER BY seq DESC LIMIT ?2",
        params: vec![json!(object), json!(n)],
    }
}

/// The Instance's event-log object, which [`event_entry`] appends to.
#[must_use]
pub fn events_object(instance: &str) -> String {
    format!("instance:{instance}/events")
}

/// The event log `rows` rebuild: their events in id order, the newest [`crate::events::KEEP`] kept, and the highest id as `last`, so a colour that adopts it keeps every session's cursor valid; see the blue-green ADR's 2026-11-20 amendments.
#[must_use]
pub fn event_log(rows: &[Row]) -> crate::events::Log {
    let mut events: Vec<crate::events::Event> = rows
        .iter()
        .filter(|r| r.kind == "event")
        .filter_map(|r| serde_json::from_value(r.body.clone()).ok())
        .collect();
    events.sort_by_key(|e| e.id);
    let last = rows.iter().map(|r| r.seq).max().unwrap_or_default();
    let skip = events.len().saturating_sub(crate::events::KEEP);
    crate::events::Log {
        last,
        list: events.into_iter().skip(skip).collect(),
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

#[cfg(test)]
mod replay_tests {
    use super::*;
    use serde_json::json;

    fn rows_of(statements: &[crate::control::Statement]) -> Vec<Row> {
        statements
            .iter()
            .map(|s| Row {
                seq: s.params[1].as_u64().unwrap(),
                kind: s.params[4].as_str().unwrap().to_string(),
                body: serde_json::from_str(s.params[5].as_str().unwrap()).unwrap(),
            })
            .collect()
    }

    #[test]
    fn every_part_row_fits_a_row_however_much_its_text_escapes() {
        let prose = json!({"briefs": "say \"hi\"\n\t\\ \u{1}é".repeat(MAX_VALUE / 4)});
        let parts = entries("o", 1, 3, 0, "briefs", &prose);
        assert!(parts.len() > 1);
        for s in &parts {
            let n = s.params[5].as_str().unwrap().len();
            assert!(n <= MAX_VALUE, "a part row is {n} bytes");
        }
        let none = |_: &str, _: &serde_json::Value| None;
        let (values, _) = restore(&rows_of(&parts), &none);
        assert_eq!(values["briefs"], prose);
    }

    #[test]
    fn a_body_longer_than_a_row_is_journaled_in_parts_and_restored_whole() {
        let big = json!({"briefs": "é".repeat(MAX_VALUE)});
        let small = entries("o", 1, 3, 0, "k", &json!(1));
        assert_eq!(small.len(), 1);
        let parts = entries("o", 2, 3, 0, "briefs", &big);
        assert_eq!(parts.len(), 3);
        for s in &parts {
            assert!(s.params[5].as_str().unwrap().len() < MAX_VALUE + 100);
        }
        let mut rows = rows_of(&small);
        rows.extend(rows_of(&parts));
        let none = |_: &str, _: &serde_json::Value| None;
        let (values, unopened) = restore(&rows, &none);
        assert_eq!(values["briefs"], big);
        assert_eq!(values["k"], json!(1));
        assert_eq!(unopened, Vec::<String>::new());
        let (values, unopened) = restore(&rows[..2], &none);
        assert!(!values.contains_key("briefs"));
        assert_eq!(unopened, vec!["briefs".to_string()]);
    }

    #[test]
    fn an_oversized_value_is_named_with_its_size() {
        use std::collections::BTreeMap;
        let mut now: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        now.insert("small".into(), json!(1));
        assert_eq!(oversized(&now), None);
        now.insert("briefs".into(), json!("x".repeat(MAX_TOTAL)));
        let why = oversized(&now).unwrap();
        assert!(why.contains("briefs: 16000002 bytes"), "{why}");
    }

    #[test]
    fn a_map_with_number_keys_is_kept_tagged_and_others_stay_plain() {
        use serde::Deserialize as _;
        use serde::de::IntoDeserializer as _;
        use std::collections::BTreeMap;
        type E = serde::de::value::Error;
        let numbered: BTreeMap<u32, String> = [(1, "b1".to_string()), (7, "b7".to_string())].into();
        let Stored(v) = Stored::deserialize(
            numbered.into_deserializer() as serde::de::value::MapDeserializer<'_, _, E>
        )
        .unwrap();
        assert_eq!(v, json!({"$map": [[1, "b1"], [7, "b7"]]}));
        let pairs = tagged_map(&v).unwrap();
        assert_eq!(
            pairs,
            vec![(&json!(1), &json!("b1")), (&json!(7), &json!("b7"))]
        );
        let named: BTreeMap<String, u32> = [("a".to_string(), 1)].into();
        let Stored(w) = Stored::deserialize(
            named.into_deserializer() as serde::de::value::MapDeserializer<'_, _, E>
        )
        .unwrap();
        assert_eq!(w, json!({"a": 1}));
        assert_eq!(tagged_map(&w), None);
        assert_eq!(tagged_map(&json!({"$map": [[1]]})), None);
    }

    #[test]
    fn a_new_epoch_journals_every_key_and_removes_each_absent_one() {
        use std::collections::BTreeMap;
        let now: BTreeMap<String, serde_json::Value> = [("a".to_string(), json!(1))].into();
        let previous: BTreeMap<String, String> = [("a".to_string(), digest(&json!(1)))].into();
        let same = baseline(previous.clone(), Some(3), 3, &["a", "b"]);
        assert_eq!(changed(&same, &now), Vec::<(String, String)>::new());
        let fresh = baseline(previous.clone(), Some(2), 3, &["a", "b"]);
        assert_eq!(
            changed(&fresh, &now),
            vec![
                ("a".to_string(), digest(&json!(1))),
                ("b".to_string(), String::new())
            ]
        );
        assert_eq!(changed(&baseline(previous, None, 3, &["a"]), &now).len(), 1);
    }

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
    fn an_event_log_is_rebuilt_in_id_order_with_its_last_id() {
        let ev = |id: u64| json!({"id": id, "at": id, "kind": "k", "summary": "s"});
        let rows = [
            row(7, "event", ev(7)),
            row(3, "event", ev(3)),
            row(5, "event", ev(5)),
        ];
        let log = event_log(&rows);
        assert_eq!(log.last, 7);
        assert_eq!(
            log.list.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![3, 5, 7]
        );
        assert_eq!(event_log(&[]).last, 0);
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
