//! The event log in the `Instance` Durable Object: appended by the control plane's objects,
//! read by sessions; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md`
//! §1.

use serde_json::{Value, json};
use std::time::Duration;

use serde_json::Map;
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::events::{Filter, Log};

/// The storage key of the log.
const LOG: &str = "events";

/// How long `events.wait` and `GET /v0/events` wait at most, in seconds.
pub const WAIT_MAX: u64 = 50;

/// How often a long poll looks.
const LOOK: Duration = Duration::from_secs(2);

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
    for (kind, summary) in found {
        if let Err(e) = append(
            env,
            &s,
            &kind,
            Some(after.number),
            after.session.clone(),
            &summary,
            false,
        )
        .await
        {
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
    let body = json!({
        "after": text("after").and_then(|a| a.parse::<u64>().ok()).unwrap_or_default(),
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

/// The `Instance` Durable Object's side: `/events/add` and `/events/since`.
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
            storage.put(LOG, &log).await?;
            Response::from_json(&json!({ "id": id }))
        }
        "/events/since" => {
            let filter = Filter {
                after: body["after"].as_u64().unwrap_or_default(),
                session: body["session"].as_str().map(str::to_string),
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
        _ => Response::error("not found", 404),
    }
}
