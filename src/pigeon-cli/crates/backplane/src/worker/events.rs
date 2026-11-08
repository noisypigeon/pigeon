//! The event log in the `Instance` Durable Object: appended by the control plane's objects,
//! read by sessions; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md`
//! §1.

use serde_json::{Value, json};
use worker::{Response, Result, State};

use super::platform::now;
use crate::events::{Filter, Log};

/// The storage key of the log.
const LOG: &str = "events";

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
