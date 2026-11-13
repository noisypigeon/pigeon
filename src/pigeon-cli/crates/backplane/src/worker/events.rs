//! The event log in the `Instance` Durable Object: appended by the control plane's objects,
//! read by sessions; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md`
//! §1.

use serde_json::{Value, json};
use std::time::Duration;

use serde_json::Map;
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::events::{After, Cursors, Filter, Log};

/// The storage key of the log.
const LOG: &str = "events";

/// How long `events.wait` and `GET /v0/events` wait at most, in seconds: under the 30 seconds
/// after which a request is cut on its way; see
/// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md`.
pub const WAIT_MAX: u64 = 25;

/// How often a long poll looks.
const LOOK: Duration = Duration::from_secs(5);

/// Appends an event to the instance's log; `owner` keeps it from agents.
///
/// # Errors
///
/// When a binding fails.
pub async fn append(
    env: &Env,
    s: &Settings,
    kind: &str,
    change: Option<u32>,
    session: Option<String>,
    summary: &str,
    owner: bool,
) -> Result<()> {
    let body = json!({
        "kind": kind, "change": change, "session": session, "summary": summary, "owner": owner,
    });
    to_instance(env, s, "/events/add", &body).await.map(|_| ())
}

/// Appends what happened to `after` since `before`, logging a failure rather than failing.
pub async fn changed(
    env: &Env,
    before: Option<&crate::changes::Change>,
    after: &crate::changes::Change,
) {
    let found = crate::events::of_change(before, after);
    if found.is_empty() {
        return;
    }
    let Ok(s) = Settings::from(env) else {
        return;
    };
    let q = super::stages::queue(env, &s).await;
    let stage = super::stages::stage(env, &s, after, &q).await;
    for (kind, summary) in found {
        let body = json!({
            "kind": kind, "change": after.number, "session": after.session,
            "summary": summary, "owner": false, "stage": stage,
        });
        if let Err(e) = to_instance(env, &s, "/events/add", &body).await {
            worker::console_error!("event {kind}: {e}");
        }
    }
}

/// The events after `after` that the filter in `args` wants, waiting up to `seconds` (at most
/// [`WAIT_MAX`]) for one: the last event's number and the events.
///
/// # Errors
///
/// When a binding fails.
pub async fn wait(
    env: &Env,
    s: &Settings,
    args: &Map<String, Value>,
    owner: bool,
) -> Result<Value> {
    let text = |k: &str| match args.get(k) {
        Some(Value::String(v)) => Some(v.clone()),
        Some(Value::Number(v)) => Some(v.to_string()),
        _ => None,
    };
    let seconds = text("seconds")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(WAIT_MAX)
        .min(WAIT_MAX);
    let changes: Vec<u32> = text("change")
        .unwrap_or_default()
        .split(',')
        .filter_map(|c| c.trim().parse().ok())
        .collect();
    let after = match text("after") {
        Some(a) if a == "session" => json!("session"),
        a => json!(a.and_then(|a| a.parse::<u64>().ok()).unwrap_or_default()),
    };
    let body = json!({
        "after": after,
        "wake": text("wake").as_deref() == Some("agent"),
        "session": text("session").filter(|v| !v.trim().is_empty()),
        "changes": changes,
        "owner": owner,
    });
    let until = now() + seconds;
    loop {
        let (_, v) = to_instance(env, s, "/events/since", &body).await?;
        let any = v["events"].as_array().is_some_and(|e| !e.is_empty());
        if any || now() >= until {
            return Ok(v);
        }
        worker::Delay::from(LOOK).await;
    }
}

/// The storage key of the sessions' cursors.
const CURSORS: &str = "event-cursors";

/// The `Instance` Durable Object's side: `/events/add`, `/events/since` and `/events/cursor`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut log: Log = storage.get(LOG).await?.unwrap_or_default();
    match path {
        "/events/add" => {
            let id = log.append(
                body["kind"].as_str().unwrap_or("event"),
                body["change"].as_u64().and_then(|c| u32::try_from(c).ok()),
                body["session"].as_str().map(str::to_string),
                body["summary"].as_str().unwrap_or_default(),
                body["owner"].as_bool() == Some(true),
                now(),
            );
            if let Some(e) = log.list.back_mut().filter(|e| e.id == id) {
                e.stage = serde_json::from_value(body["stage"].clone()).ok();
            }
            storage.put(LOG, &log).await?;
            Response::from_json(&json!({ "id": id }))
        }
        "/events/since" => {
            let session = body["session"].as_str().map(str::to_string);
            let asked = match body["after"].as_str() {
                Some("session") => After::Session,
                _ => After::At(body["after"].as_u64().unwrap_or_default()),
            };
            let after = match &session {
                Some(label) => {
                    let mut cursors: Cursors = storage.get(CURSORS).await?.unwrap_or_default();
                    let after = cursors.poll(label, asked, log.last, now());
                    storage.put(CURSORS, &cursors).await?;
                    after
                }
                None => match asked {
                    After::Session => log.last,
                    After::At(n) => n,
                },
            };
            let filter = Filter {
                after,
                wake: body["wake"].as_bool() == Some(true),
                session,
                changes: body["changes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_u64().and_then(|c| u32::try_from(c).ok()))
                    .collect(),
                owner: body["owner"].as_bool() == Some(true),
            };
            Response::from_json(&json!({ "last": log.last, "events": log.since(&filter) }))
        }
        "/events/cursor" => {
            let cursors: Cursors = storage.get(CURSORS).await?.unwrap_or_default();
            let label = body["session"].as_str().unwrap_or_default();
            Response::from_json(&match cursors.listening(label, now()) {
                Some((c, listening)) => {
                    json!({ "after": c.after, "polled": c.polled, "listening": listening })
                }
                None => json!({ "listening": false }),
            })
        }
        _ => Response::error("not found", 404),
    }
}
