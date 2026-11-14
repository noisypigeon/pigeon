//! Tasks in the control plane: the `task.*` operations and the `Instance` Durable Object's store
//! of them; see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.

use std::time::Duration;

use serde_json::{Map, Value, json};
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::sessions::Class;
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

/// `session`'s class, from its ticket: opus for none or a label never started; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.
async fn class(env: &Env, s: &Settings, session: Option<&str>) -> Result<Class> {
    let Some(label) = session else {
        return Ok(Class::Opus);
    };
    let (_, t) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(t).unwrap_or_default();
    Ok(tickets.class(label).unwrap_or_default())
}

/// Why an executor of `class` may not take task `id`, from its kind and its change's brief and
/// scope; see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §2.
async fn executor_refused(
    env: &Env,
    settings: &Settings,
    session: Option<&str>,
    class: Class,
    id: Option<u32>,
) -> Result<Option<String>> {
    use crate::core::brief::executor::claim_refused;
    let body = json!({ "session": session, "class": class });
    let (_, listed) = to_instance(env, settings, "/tasks/list", &body).await?;
    let Some(task) = listed["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"].as_u64().and_then(|n| u32::try_from(n).ok()) == id)
    else {
        return Ok(None);
    };
    let kind = task["kind"].as_str().unwrap_or_default();
    let number = task["change"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0);
    let Some(number) = number else {
        return Ok(claim_refused(kind, None, &[]));
    };
    let (_, status) = super::to_change(
        env,
        &settings.repo,
        number,
        worker::Method::Get,
        "/status",
        None,
    )
    .await?;
    let scope: Vec<String> =
        serde_json::from_value(status["change"]["scope"].clone()).unwrap_or_default();
    Ok(claim_refused(
        kind,
        status["change"]["brief"].as_str(),
        &scope,
    ))
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
    let class = class(env, s, session.as_deref()).await?;
    match operation {
        "task.list" => {
            let body = json!({ "session": session, "class": class });
            to_instance(env, s, "/tasks/list", &body).await
        }
        "task.claim" => {
            if class != Class::Opus
                && let Some(why) = executor_refused(env, s, session.as_deref(), class, id).await?
            {
                return Ok((403, json!(why)));
            }
            let (status, v) = to_instance(
                env,
                s,
                "/tasks/claim",
                &json!({ "id": id, "session": session, "class": class }),
            )
            .await?;
            if status == 200 && v["kind"] == "handoff" {
                handed(env, s, &v, session.as_deref().unwrap_or_default()).await?;
            }
            Ok((status, v))
        }
        "task.done" => to_instance(env, s, "/tasks/done", &json!({ "id": id })).await,
        "task.wait" => {
            let seconds = text(args, "seconds")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(WAIT_MAX)
                .min(WAIT_MAX);
            let until = now() + seconds;
            loop {
                let body = json!({ "session": session, "class": class });
                let (status, v) = to_instance(env, s, "/tasks/list", &body).await?;
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
    to_instance(env, s, "/tasks/add", &body).await?;
    let said = format!("task ({kind}) on !{change}: {note}");
    super::events::append(env, s, "task.made", Some(change), session, &said, false).await
}

/// A claimed handoff task `t`: its change becomes `session`'s; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.
async fn handed(env: &Env, s: &Settings, t: &Value, session: &str) -> Result<()> {
    let Some(n) = t["change"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
        return Ok(());
    };
    let body = json!({ "session": session });
    super::to_change(
        env,
        &s.repo,
        n,
        worker::Method::Post,
        "/session",
        Some(&body),
    )
    .await?;
    let said = format!("!{n} is now {session}'s");
    super::events::append(
        env,
        s,
        "change.handed",
        Some(n),
        Some(session.to_string()),
        &said,
        false,
    )
    .await
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
    let why = c
        .violations
        .last()
        .map_or(note.clone(), |v| format!("{note}: {v}"));
    let said = super::events::append(
        env,
        &s,
        "restack.failed",
        Some(c.number),
        c.session.clone(),
        &why,
        false,
    );
    if let Err(e) = said.await {
        worker::console_error!("restack.failed: {e}");
    }
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

/// The `Instance` Durable Object's side: `/tasks/add`, `list`, `claim`, `done`, `close` and
/// `sweep`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Tasks = storage.get(TASKS).await?.unwrap_or_default();
    let n = |k: &str| body[k].as_u64().and_then(|v| u32::try_from(v).ok());
    let session = body["session"].as_str();
    let class: Class = serde_json::from_value(body["class"].clone()).unwrap_or_default();
    let answer: std::result::Result<Value, String> = match path {
        "/tasks/add-brief" => match (
            body["brief"].as_str(),
            serde_json::from_value(body["model"].clone()),
        ) {
            (Some(b), Ok(m)) => Ok(
                json!({ "id": all.add_brief(b, m, body["note"].as_str().unwrap_or_default(), now()) }),
            ),
            _ => Err("a brief task names its `brief` and `model`".into()),
        },
        "/tasks/add" => Ok(json!({ "id": all.add(
            body["kind"].as_str().unwrap_or("task"),
            n("change").unwrap_or_default(),
            n("parent"),
            session.map(str::to_string),
            body["note"].as_str().unwrap_or_default(),
            now(),
        ) })),
        "/tasks/list" => {
            return Response::from_json(&json!({ "tasks": all.visible_as(session, class, now()) }));
        }
        "/tasks/claim" => match n("id") {
            Some(id) => all
                .claim_as(id, session.unwrap_or_default(), class, now())
                .map(|t| json!(t)),
            None => Err("`id` must be a task number".into()),
        },
        "/tasks/done" => match n("id") {
            Some(id) => all.done(id).map(|t| json!(t)),
            None => Err("`id` must be a task number".into()),
        },
        "/tasks/close" => Ok(json!({ "closed": all.close_for(n("change").unwrap_or_default()) })),
        "/tasks/sweep" => {
            let open = serde_json::from_value(body["open"].clone()).unwrap_or_default();
            Ok(json!({ "closed": all.sweep(&open) }))
        }
        _ => return Response::error("not found", 404),
    };
    storage.put(TASKS, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}
