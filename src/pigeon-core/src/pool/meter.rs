//! The metering route's pure decisions: task tokens, vetting a request, and settling its
//! response; see `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md` §4
//! and its 2026-11-17 amendment.

use std::collections::BTreeSet;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::Micros;
use super::aigateway::{self, Meta};
use super::config::Pool;
use super::gateway::{Gateway, Usage};
use super::ledger::{self, Ledger, Signal, UNRECONCILED};
use crate::change::{Change, State};
use crate::provider::verdict::{self, Caller, ChangeState, Ticket};
use crate::push::{b64url, b64url_decode};

/// The label `SESSION_KEY` is derived under for task tokens.
pub const TASK_KEY_LABEL: &str = "pigeon-pool-task-v1";
/// The capability a pool task's caller holds for model requests.
pub const CAPABILITY: &str = "pool.messages";
/// The class's helper model, admitted beside the task's own.
pub const HELPER_MODEL: &str = "claude-haiku-5-5";

fn mac(key: &[u8], text: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    m.update(text.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// The task-token key derived from `session_key`.
#[must_use]
pub fn task_key(session_key: &[u8]) -> Vec<u8> {
    mac(session_key, TASK_KEY_LABEL)
}

/// A task token: `<pool>.<task>.<expires>.<signature>`, the signature base64url.
#[must_use]
pub fn sign_task(key: &[u8], pool: &str, task: &str, expires: u64) -> String {
    let body = format!("{pool}.{task}.{expires}");
    format!("{body}.{}", b64url(&mac(key, &body)))
}

/// The pool and task `token` names, if `key` signed it and it lives at `now`.
///
/// # Errors
///
/// Why it isn't a live task token.
pub fn verify_task(key: &[u8], token: &str, now: u64) -> Result<(String, String), String> {
    let bad = || "pool: not a task token".to_string();
    let (body, sig) = token.trim().rsplit_once('.').ok_or_else(bad)?;
    let (rest, expires) = body.rsplit_once('.').ok_or_else(bad)?;
    let (pool, task) = rest.split_once('.').ok_or_else(bad)?;
    if pool.is_empty() || task.is_empty() {
        return Err(bad());
    }
    let wanted = mac(key, body);
    let got = b64url_decode(sig).unwrap_or_default();
    let same = got.len() == wanted.len()
        && got
            .iter()
            .zip(&wanted)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if !same {
        return Err(bad());
    }
    let expires: u64 = expires.parse().map_err(|_| bad())?;
    if expires <= now {
        return Err(format!("pool: task {task}'s token has expired"));
    }
    Ok((pool.to_string(), task.to_string()))
}

/// What the `Pool` object keeps about an open task beside the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskInfo {
    /// Its session label, `pool:b<n>.<k>`.
    pub label: String,
    /// Its change.
    pub change: u64,
    /// Its brief, `b<n>`.
    pub brief: String,
    /// Its class, `haiku` or `sonnet`.
    pub class: String,
    /// Its model.
    pub model: String,
}

/// The caller a pool task's request is checked as: its label's ticket and the changes it holds.
#[must_use]
pub fn caller(label: &str, ticket: Ticket, holds: BTreeSet<u64>) -> Caller {
    Caller {
        account: label.to_string(),
        platform: false,
        capabilities: BTreeSet::from([CAPABILITY.to_string()]),
        ticket,
        holds,
    }
}

/// A change's state as the broker's verdict reads it: an eviction counts though the change is open.
#[must_use]
pub fn change_state(c: &Change) -> ChangeState {
    if c.evicted.is_some() {
        return ChangeState::Evicted;
    }
    match c.state {
        State::Merged => ChangeState::Merged,
        State::Closed => ChangeState::Closed,
        _ => ChangeState::Open,
    }
}

/// A request the route forwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    /// The ledger with the request's hold.
    pub ledger: Ledger,
    /// What it reserved.
    pub hold: Micros,
    /// Its `cf-aig-metadata`.
    pub meta: Meta,
    /// Whether its response streams.
    pub stream: bool,
}

/// A request the route refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refuse {
    /// The status answered.
    pub status: u16,
    /// Why, starting `pool: `.
    pub why: String,
    /// The outcome the task closes with, if the refusal ends it.
    pub close: Option<&'static str>,
}

fn refuse(status: u16, why: impl Into<String>) -> Refuse {
    Refuse {
        status,
        why: why.into(),
        close: None,
    }
}

/// What a request is checked against.
pub struct Vet<'a> {
    /// The pool's gateway.
    pub gateway: &'a dyn Gateway,
    /// The pool.
    pub pool: &'a Pool,
    /// Its ledger.
    pub ledger: &'a Ledger,
    /// The pinned workspace, if set.
    pub workspace: Option<&'a str>,
    /// The task the token named.
    pub task: &'a str,
    /// What the object keeps about it.
    pub info: Option<&'a TaskInfo>,
    /// The caller, from the task's label.
    pub caller: &'a Caller,
    /// The task's change's state.
    pub change: ChangeState,
}

/// Vets one request, in the amendment's order, and reserves it as hold `hold`.
///
/// # Errors
///
/// The [`Refuse`] the route answers.
pub fn vet(v: &Vet<'_>, hold: &str, path: &str, body: &[u8], now: u64) -> Result<Forward, Refuse> {
    if !v.pool.enabled {
        return Err(refuse(403, "pool: disabled"));
    }
    if v.workspace.is_none_or(str::is_empty) {
        return Err(refuse(503, "pool: no pinned workspace"));
    }
    let Some(info) = v.info else {
        return Err(refuse(403, format!("pool: no task {}", v.task)));
    };
    if let Err((step, reason)) = verdict::admit(v.caller, CAPABILITY, Some((info.change, v.change)))
    {
        return Err(Refuse {
            status: 403,
            why: format!(
                "pool: change {} evicted ({}: {reason})",
                info.change,
                step.name()
            ),
            close: Some("stopped (evicted)"),
        });
    }
    let models = [info.model.clone(), HELPER_MODEL.to_string()];
    let admitted = v
        .gateway
        .admit(path, body, &models)
        .map_err(|r| refuse(400, r.0))?;
    let amount = v.gateway.reserve(&admitted, v.pool);
    let ledger = ledger::reserve(v.ledger, v.pool, v.task, hold, amount, now).map_err(|r| {
        let close = match r {
            ledger::Refusal::Spent { .. } | ledger::Refusal::Deadline(_) => Some("stopped"),
            _ => None,
        };
        Refuse {
            status: 403,
            why: r.to_string(),
            close,
        }
    })?;
    let meta = Meta {
        pool: v.pool.name.clone(),
        task: v.task.to_string(),
        change: info.change,
        brief: info.brief.clone(),
        class: info.class.clone(),
    };
    Ok(Forward {
        ledger,
        hold: amount,
        meta,
        stream: admitted.stream,
    })
}

/// A response as the route saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen<'a> {
    /// Its status; 0 when the gateway couldn't be reached.
    pub status: u16,
    /// The account header's value, if any.
    pub account: Option<&'a str>,
    /// Its body, for an error's signals; empty for a stream.
    pub body: &'a str,
    /// Its final usage, if it gave one.
    pub usage: Option<Usage>,
}

/// What settling a response did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// The ledger after it.
    pub ledger: Ledger,
    /// What was charged.
    pub cost: Micros,
    /// Why it paused the pool, once per tripwire; each files an owner item.
    pub trips: Vec<String>,
}

/// The gateway's signal as a tripwire, or None when it doesn't pause the pool.
fn gateway_trip(s: aigateway::Signal) -> Option<String> {
    s.pauses().then(|| match s {
        aigateway::Signal::GatewayNoKey => "the AI Gateway has no stored key".to_string(),
        aigateway::Signal::GatewayUnauthorized => "the AI Gateway refused the token".to_string(),
        aigateway::Signal::Gateway(0) => "the AI Gateway is unavailable".to_string(),
        aigateway::Signal::Gateway(c) => format!("the AI Gateway answered error {c}"),
        _ => unreachable!(),
    })
}

/// Charges hold `hold` for response `seen`, applying its signals: a response without a final
/// usage is charged its reservation, and any other workspace trips.
#[must_use]
pub fn settle(
    gateway: &dyn Gateway,
    pool: &Pool,
    l: &Ledger,
    hold: &str,
    workspace: &str,
    seen: &Seen<'_>,
    now: u64,
) -> Settled {
    let mut trips = Vec::new();
    let mut n = l.clone();
    let reserved = l.holds.get(hold).map_or(0, |h| h.amount);
    let ok = (200..300).contains(&seen.status);
    let mut apply = |n: &Ledger, s: Signal| {
        let (m, trip) = ledger::signal(n, s);
        trips.extend(trip.map(|t| t.why));
        m
    };
    if ok && gateway.account_header().is_some() && seen.account != Some(workspace) {
        n = apply(&n, Signal::WrongAccount);
    }
    if !ok {
        match aigateway::classify(seen.status, seen.body) {
            Some(aigateway::Signal::Upstream) => {
                n = apply(&n, gateway.classify(seen.status, seen.body));
            }
            Some(s) => {
                if let Some(why) = gateway_trip(s) {
                    n = ledger::pause(&n, &why);
                    trips.push(why);
                }
            }
            None => {}
        }
    }
    let final_usage = seen.usage.as_ref().filter(|u| u.complete);
    let (cost, fresh, model) = match final_usage {
        Some(u) => (
            gateway.cost(u, pool),
            gateway.fresh_tokens(u),
            u.model.clone(),
        ),
        None => (reserved, 0, UNRECONCILED.to_string()),
    };
    let charged = match ledger::reconcile(&n, hold, cost, fresh, &model, now) {
        Ok((m, trip)) => {
            trips.extend(trip.map(|t| t.why));
            m
        }
        Err(_) => n,
    };
    if final_usage.is_some_and(|u| !pool.prices.contains_key(&u.model)) {
        let why = format!("a response named model `{model}`, outside the price table");
        let paused = ledger::pause(&charged, &why);
        trips.push(why);
        return Settled {
            ledger: paused,
            cost,
            trips,
        };
    }
    Settled {
        ledger: charged,
        cost,
        trips,
    }
}

/// A stream's usage, read from its SSE `data:` lines as chunks arrive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sse {
    line: Vec<u8>,
    /// The usage so far.
    pub usage: Usage,
}

/// The longest line kept; a longer one is dropped, as no usage event is that long.
pub const MAX_LINE: usize = 1 << 20;

impl Sse {
    /// Reads `chunk`, which may split a line or a character.
    pub fn feed(&mut self, gateway: &dyn Gateway, chunk: &[u8]) {
        for &b in chunk {
            if b == b'\n' {
                let line = std::mem::take(&mut self.line);
                let text = String::from_utf8_lossy(&line);
                let text = text.trim_end_matches('\r');
                if let Some(data) = text.strip_prefix("data:") {
                    self.usage = gateway.usage_event(&self.usage, data.trim_start());
                }
            } else if self.line.len() < MAX_LINE {
                self.line.push(b);
            }
        }
    }
}

/// The Cloudflare error code in a gateway answer's body, if any.
fn cloudflare_code(body: &str) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let list = v
        .get("error")
        .and_then(serde_json::Value::as_array)
        .or_else(|| v.get("errors").and_then(serde_json::Value::as_array))?;
    list.first()?.get("code")?.as_i64()
}

/// The `pool.upstream` event's fields for answer `seen` to `task`: every non-2xx answer, and a
/// 2xx answer when it is the task's `first`; the status, whether the workspace header came
/// back, the signals and any Cloudflare code, never a body, header value or token; see the
/// capacity pools ADR's 2026-11-17 amendments.
#[must_use]
pub fn upstream_event(
    gateway: &dyn Gateway,
    pool: &str,
    task: &str,
    seen: &Seen<'_>,
    first: bool,
) -> Option<serde_json::Value> {
    let ok = (200..300).contains(&seen.status);
    if ok && !first {
        return None;
    }
    let mut e = serde_json::json!({
        "pool": pool,
        "task": task,
        "status": seen.status,
        "workspace_seen": seen.account.is_some(),
    });
    if ok {
        return Some(e);
    }
    let signal = match aigateway::classify(seen.status, seen.body) {
        None => "ok".to_string(),
        Some(aigateway::Signal::Upstream) => {
            let s = gateway.classify(seen.status, seen.body);
            e["anthropic"] = serde_json::to_value(s).unwrap_or_default();
            "upstream".to_string()
        }
        Some(aigateway::Signal::GatewayLimited) => "gateway-limited".to_string(),
        Some(aigateway::Signal::GatewayNoKey) => "gateway-no-key".to_string(),
        Some(aigateway::Signal::GatewayUnauthorized) => "gateway-unauthorized".to_string(),
        Some(aigateway::Signal::Gateway(c)) => format!("gateway({c})"),
    };
    e["signal"] = signal.into();
    if let Some(c) = cloudflare_code(seen.body) {
        e["code"] = c.into();
    }
    Some(e)
}

#[cfg(test)]
mod tests;
