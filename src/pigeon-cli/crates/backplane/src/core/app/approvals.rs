//! The app's view of waiting approvals: each named by its grant's key, so the grant's token
//! never leaves the Worker; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §3, §4.

use serde_json::{Value, json};

use crate::approve::Waiting;
use crate::core::approval::grants::{self, Grant};
use crate::core::change::Change;

/// The token of a waiting change approval, from its page's path.
#[must_use]
pub fn token(w: &Waiting) -> Option<&str> {
    if !w.path.starts_with("/approve/") {
        return None;
    }
    w.path.split_once("?grant=").map(|(_, t)| t)
}

/// The ID the app knows a waiting approval by: its grant's key.
#[must_use]
pub fn id(w: &Waiting) -> Option<String> {
    token(w).map(grants::key)
}

/// The waiting change approval named `id`, with its token.
#[must_use]
pub fn find<'a>(waiting: &'a [Waiting], id: &str) -> Option<(&'a Waiting, &'a str)> {
    waiting
        .iter()
        .find_map(|w| token(w).filter(|t| grants::key(t) == id).map(|t| (w, t)))
}

/// The app's list: each waiting change approval, newest first.
#[must_use]
pub fn list(waiting: &[Waiting]) -> Value {
    let mut items: Vec<&Waiting> = waiting.iter().filter(|w| token(w).is_some()).collect();
    items.sort_by_key(|w| std::cmp::Reverse(w.expires));
    Value::Array(
        items
            .into_iter()
            .filter_map(|w| {
                Some(json!({
                    "id": id(w)?, "number": w.number, "operation": w.operation,
                    "title": w.title, "expires": w.expires,
                }))
            })
            .collect(),
    )
}

/// One approval's detail: what the card shows.
#[must_use]
pub fn detail(w: &Waiting, change: &Change, grant: &Grant) -> Value {
    let checks: Vec<Value> = change
        .jobs
        .iter()
        .map(|j| json!({ "name": j.name, "status": j.status }))
        .collect();
    let tier = change.tiers.as_ref().map(|t| {
        if w.operation == "change.authorize" {
            &t.authorize
        } else {
            &t.merge
        }
    });
    json!({
        "id": id(w), "number": change.number, "operation": w.operation,
        "title": change.title, "branch": change.branch(), "head": grant.head,
        "state": change.state.name(), "scope": change.scope, "plan": change.plan,
        "checks": checks, "tier": tier.map(|t| &t.tier), "rule": tier.map(|t| &t.rule),
        "proposer": change.proposer, "expires": w.expires,
        "needs_proof": grant.needs_code(),
        "asks": if w.operation == "change.authorize" { change.asks.as_slice() } else { &[] },
    })
}

/// The answers in the app's body for `change`'s asks, by key; anything else is dropped.
#[must_use]
pub fn answers(change: &Change, body: &Value) -> std::collections::BTreeMap<String, String> {
    change
        .asks
        .iter()
        .filter_map(|a| {
            let v = body["answers"][&a.key].as_str()?;
            Some((a.key.clone(), v.trim().to_string()))
        })
        .collect()
}

/// The passkey answer's fields as the approval form posts them, from the app's JSON.
#[must_use]
pub fn proof_form(body: &Value) -> String {
    ["id", "client", "authenticator", "signature"]
        .iter()
        .map(|k| {
            let v = body["passkey"][k].as_str().unwrap_or_default();
            let safe: String = v
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            format!("pk_{k}={safe}")
        })
        .collect::<Vec<_>>()
        .join("&")
}
