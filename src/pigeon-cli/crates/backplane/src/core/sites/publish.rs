//! What publishing a site puts in its bucket: each key's body and type, and what a sync
//! changes, decided from what the shell read and listed; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §7.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::cobot::generations::Day;
use crate::cobot::s3::{self, Credentials};
use crate::cobot::{apex, static_site};

pub use super::view::content_type;

/// One object to upload: its body and type.
#[derive(Debug, Clone)]
pub struct Upload {
    /// The body.
    pub body: Vec<u8>,
    /// Its `Content-Type`.
    pub content_type: &'static str,
}

/// The key holding each object's last written `Content-Type`; a site may not use it.
pub const TYPES: &str = ".cobot/types.json";

/// Every key the bucket should hold, from the files under `public` by path (`files`): each
/// file, plus `x` and `x/` copies of each `x/index.html` below the root.
///
/// # Errors
///
/// A message when a file sits at [`TYPES`].
pub fn expected_of(files: BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, Upload>, String> {
    if files.contains_key(TYPES) {
        return Err(format!(
            "`{TYPES}` is cobot's own key in the bucket; a site cannot publish it"
        ));
    }
    let mut out: BTreeMap<String, Upload> = files
        .into_iter()
        .map(|(key, body)| {
            let content_type = content_type(&key);
            (key, Upload { body, content_type })
        })
        .collect();
    let pages: Vec<(String, Upload)> = out
        .iter()
        .filter_map(|(k, v)| {
            k.strip_suffix("/index.html")
                .map(|dir| (dir.to_string(), v.clone()))
        })
        .collect();
    for (dir, page) in pages {
        let html = Upload {
            body: page.body,
            content_type: content_type("index.html"),
        };
        out.insert(format!("{dir}/"), html.clone());
        out.insert(dir, html);
    }
    Ok(out)
}

/// What a sync did, by key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Synced {
    /// Keys written, new or changed.
    pub put: Vec<String>,
    /// Keys left as they were.
    pub unchanged: usize,
    /// Keys deleted, no longer expected.
    pub deleted: Vec<String>,
}

/// What a sync does: the keys to put, how many stay, the types record to write, if it changed,
/// and the keys to delete.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    /// Keys to write, new or changed, in order.
    pub put: Vec<String>,
    /// Keys left as they were.
    pub unchanged: usize,
    /// The [`TYPES`] record's new body, when it changed.
    pub types: Option<Vec<u8>>,
    /// Keys to delete, no longer expected.
    pub delete: Vec<String>,
}

/// How to make a bucket that lists `have` (key to `ETag`) and last wrote `written` (key to
/// `Content-Type`) hold exactly `expected`: a key is put unless its MD5 and type both match.
#[must_use]
pub fn sync_plan(
    expected: &BTreeMap<String, Upload>,
    have: &BTreeMap<String, String>,
    written: &BTreeMap<String, String>,
) -> SyncPlan {
    let types: BTreeMap<String, String> = expected
        .iter()
        .map(|(k, u)| (k.clone(), u.content_type.to_string()))
        .collect();
    let mut plan = SyncPlan::default();
    for (key, up) in expected {
        if have.get(key).is_some_and(|e| *e == s3::md5_hex(&up.body))
            && written.get(key).is_some_and(|t| t == up.content_type)
        {
            plan.unchanged += 1;
        } else {
            plan.put.push(key.clone());
        }
    }
    if *written != types {
        plan.types = Some(serde_json::to_vec(&types).unwrap_or_default());
    }
    let keep: BTreeSet<&str> = expected.keys().map(String::as_str).chain([TYPES]).collect();
    plan.delete = have
        .keys()
        .filter(|k| !keep.contains(k.as_str()))
        .cloned()
        .collect();
    plan
}

/// The site token's credentials and the account id, from `infra/cloudflare`'s
/// `tofu show -json` state: the live generation's id as the access key, and
/// the SHA-256 of its value as the secret.
#[must_use]
pub fn credentials_of(
    shown: &str,
    zone: &str,
    workload: &str,
    today: Day,
) -> Option<(Credentials, String)> {
    let state: Value = serde_json::from_str(shown).ok()?;
    let outputs = &state["values"]["outputs"];
    let token = static_site::token_name(zone, workload);
    let (g, value) = apex::live_token(outputs, "tokens", &token, today)?;
    let id = outputs["token_ids"]["value"][g.key(&token)].as_str()?;
    let account = outputs["account_id"]["value"].as_str()?;
    Some((
        Credentials {
            access_key: id.to_string(),
            secret_key: s3::sha256_hex(value.as_bytes()),
        },
        account.to_string(),
    ))
}

/// R2's S3 endpoint for an account, in the EU jurisdiction.
#[must_use]
pub fn endpoint(account: &str) -> String {
    format!("https://{account}.eu.r2.cloudflarestorage.com")
}
