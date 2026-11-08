//! Tasks in the control plane: the `task.*` operations and the `Instance` Durable Object's store
//! of them; see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.

use std::time::Duration;

use serde_json::{Map, Value, json};
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::tasks::{Tasks, WAIT_MAX};

/// The storage key of every task.
const TASKS: &str = "tasks";

/// How often `task.wait` looks.
const LOOK: Duration = Duration::from_secs(5);

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Runs `operation` with `args`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    let session = text(args, "session").filter(|v| !v.trim().is_empty());
    let id = text(args, "id").and_then(|i| i.parse::<u32>().ok());
    match operation {
        "task.list" => to_instance(env, s, "/tasks/list", &json!({ "session": session })).await,
        "task.claim" => {
            to_instance(
                env,
                s,
                "/tasks/claim",
                &json!({ "id": id, "session": session }),
            )
            .await
        }
        "task.done" => to_instance(env, s, "/tasks/done", &json!({ "id": id })).await,
        "task.wait" => {
            let seconds = text(args, "seconds")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(WAIT_MAX)
                .min(WAIT_MAX);
            let until = now() + seconds;
            loop {
                let (status, v) =
                    to_instance(env, s, "/tasks/list", &json!({ "session": session })).await?;
                let open = v["tasks"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|t| t["claimed_by"].is_null()));
                if status != 200 || open || now() >= until {
                    return Ok((status, v));
                }
                worker::Delay::from(LOOK).await;
            }
        }
        other => Ok((404, json!(format!("`{other}` is not a task operation")))),
    }
}

/// Adds a task of `kind` on `change`, for `session` first.
///
/// # Errors
///
/// When a binding fails.
pub async fn add(
    env: &Env,
    s: &Settings,
    kind: &str,
    change: u32,
    parent: Option<u32>,
    session: Option<String>,
    note: &str,
) -> Result<()> {
    let body = json!({
        "kind": kind, "change": change, "parent": parent, "session": session, "note": note,
    });
    to_instance(env, s, "/tasks/add", &body).await.map(|_| ())
}

/// Tells `c`'s session its restack onto `main` failed.
///
/// # Errors
///
/// When a binding fails.
pub async fn restack_failed(env: &Env, c: &crate::changes::Change) -> Result<()> {
    let s = Settings::from(env)?;
    let note = format!(
        "{}'s restack onto main failed; rebase it onto main and push",
        c.branch()
    );
    add(
        env,
        &s,
        "restack-failed",
        c.number,
        None,
        c.session.clone(),
        &note,
    )
    .await
}

/// Closes the open tasks on change `n`.
///
/// # Errors
///
/// When a binding fails.
pub async fn close_for(env: &Env, s: &Settings, n: u32) -> Result<()> {
    to_instance(env, s, "/tasks/close", &json!({ "change": n }))
        .await
        .map(|_| ())
}

/// The `Instance` Durable Object's side: `/tasks/add`, `list`, `claim`, `done` and `close`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Tasks = storage.get(TASKS).await?.unwrap_or_default();
    let n = |k: &str| body[k].as_u64().and_then(|v| u32::try_from(v).ok());
    let session = body["session"].as_str();
    let answer: std::result::Result<Value, String> = match path {
        "/tasks/add" => Ok(json!({ "id": all.add(
            body["kind"].as_str().unwrap_or("task"),
            n("change").unwrap_or_default(),
            n("parent"),
            session.map(str::to_string),
            body["note"].as_str().unwrap_or_default(),
            now(),
        ) })),
        "/tasks/list" => {
            return Response::from_json(&json!({ "tasks": all.visible(session, now()) }));
        }
        "/tasks/claim" => match n("id") {
            Some(id) => all
                .claim(id, session.unwrap_or_default(), now())
                .map(|t| json!(t)),
            None => Err("`id` must be a task number".into()),
        },
        "/tasks/done" => match n("id") {
            Some(id) => all.done(id).map(|t| json!(t)),
            None => Err("`id` must be a task number".into()),
        },
        "/tasks/close" => Ok(json!({ "closed": all.close_for(n("change").unwrap_or_default()) })),
        _ => return Response::error("not found", 404),
    };
    storage.put(TASKS, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}
