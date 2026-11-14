//! Briefs in the control plane: the `brief.*` operations and the `Instance` Durable Object's
//! store of them; see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use serde_json::{Map, Value, json};
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::briefs::Briefs;
use crate::core::brief::Draft;
use crate::sessions::Class;
use crate::tickets::Tickets;

/// The storage key of every brief.
const BRIEFS: &str = "briefs";

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The `draft` argument: an object, or one as JSON text.
fn draft(args: &Map<String, Value>) -> std::result::Result<Draft, String> {
    let v = match args.get("draft") {
        Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| e.to_string())?,
        Some(v) => v.clone(),
        None => return Err("`draft` is the brief's content".into()),
    };
    serde_json::from_value(v).map_err(|e| format!("`draft`: {e}"))
}

/// `session`, when its ticket's class plans: opus, or a label never started.
async fn planner(
    env: &Env,
    s: &Settings,
    args: &Map<String, Value>,
) -> Result<std::result::Result<String, String>> {
    let Some(label) = text(args, "session") else {
        return Ok(Err("`session` is the planner's label".into()));
    };
    let (_, t) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: Tickets = serde_json::from_value(t).unwrap_or_default();
    let class = tickets.class(&label).unwrap_or_default();
    Ok(if class == Class::Opus {
        Ok(label)
    } else {
        Err(format!(
            "session {label} is {}, an executor; only an opus session writes briefs",
            class.name()
        ))
    })
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
    match operation {
        "brief.show" => {
            to_instance(env, s, "/briefs/show", &json!({ "id": text(args, "id") })).await
        }
        "brief.list" => {
            let body = json!({ "planner": text(args, "planner") });
            to_instance(env, s, "/briefs/list", &body).await
        }
        "brief.create" | "brief.amend" => {
            let label = match planner(env, s, args).await? {
                Ok(l) => l,
                Err(why) => return Ok((403, json!(why))),
            };
            let d = match draft(args) {
                Ok(d) => d,
                Err(why) => return Ok((400, json!(why))),
            };
            let (path, kind) = if operation == "brief.create" {
                ("/briefs/create", "brief.created")
            } else {
                ("/briefs/amend", "brief.amended")
            };
            let body = json!({
                "planner": label, "draft": d, "id": text(args, "id"), "reason": text(args, "reason"),
            });
            let (status, v) = to_instance(env, s, path, &body).await?;
            if status == 200 {
                let said = format!(
                    "{} r{} ({}): {}",
                    v["id"].as_str().unwrap_or_default(),
                    v["revisions"].as_array().map_or(0, Vec::len),
                    d.model.name(),
                    d.title
                );
                super::events::append(env, s, kind, None, Some(label), &said, false).await?;
            }
            Ok((status, v))
        }
        other => Ok((404, json!(format!("`{other}` is not a brief operation")))),
    }
}

/// The `Instance` Durable Object's side: `/briefs/create`, `/briefs/amend`, `/briefs/show` and
/// `/briefs/list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Briefs = storage.get(BRIEFS).await?.unwrap_or_default();
    let planner = body["planner"].as_str().unwrap_or_default();
    let id = body["id"].as_str().unwrap_or_default();
    let draft =
        || serde_json::from_value::<Draft>(body["draft"].clone()).map_err(|e| vec![e.to_string()]);
    let answer = match path {
        "/briefs/show" => {
            return match all.get(id) {
                Some(b) => Response::from_json(b),
                None => Response::error(format!("no brief `{id}`"), 404),
            };
        }
        "/briefs/list" => {
            let p = body["planner"].as_str();
            let rows: Vec<Value> = all
                .list(p)
                .into_iter()
                .map(|b| {
                    let r = b.latest();
                    json!({
                        "id": b.id, "planner": b.planner, "revision": b.revision(),
                        "model": r.draft.model, "title": r.draft.title,
                        "adr": r.draft.adr, "step": r.draft.step, "at": r.at,
                    })
                })
                .collect();
            return Response::from_json(&json!({ "briefs": rows }));
        }
        "/briefs/create" => draft().and_then(|d| all.create(planner, d, now())),
        "/briefs/amend" => {
            let reason = body["reason"].as_str().unwrap_or_default();
            draft().and_then(|d| all.amend(id, planner, d, reason, now()))
        }
        _ => return Response::error("not found", 404),
    };
    match answer {
        Ok(b) => {
            storage.put(BRIEFS, &all).await?;
            Response::from_json(&b)
        }
        Err(problems) => Response::error(problems.join("; "), 422),
    }
}
