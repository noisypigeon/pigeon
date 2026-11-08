//! Mandates in the control plane: the `mandate.*` operations, the `Instance` Durable Object's
//! store of them, and `/mandate/<id>`, the page that grants one with the owner's code; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §3.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::platform::{self, Settings, json_request, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::changes::Change;
use crate::mandates::{Decision, Mandate, Mandates};

/// The storage key of every mandate.
const MANDATES: &str = "mandates";

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub(super) async fn to_instance(
    env: &Env,
    s: &Settings,
    path: &str,
    body: &Value,
) -> Result<(u16, Value)> {
    let req = json_request(
        &format!("https://instance{path}"),
        Method::Post,
        Some(body),
        None,
    )?;
    let stub = env
        .durable_object("INSTANCE")?
        .id_from_name(&s.instance)?
        .get_stub()?;
    let mut r = platform::call(env, &stub, req).await?;
    let status = r.status_code();
    let text = r.text().await?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    ))
}

/// Runs `operation` with `args` for `who`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    match operation {
        "mandate.propose" => {
            let body = json!({
                "adr": text(args, "adr").unwrap_or_default(),
                "summary": text(args, "summary").unwrap_or_default(),
                "scope": text(args, "scope").unwrap_or_default().lines().collect::<Vec<_>>(),
                "may_deploy": text(args, "may_deploy").as_deref() == Some("true"),
                "budget": text(args, "budget").and_then(|b| b.parse::<u32>().ok()),
                "hours": text(args, "hours").and_then(|h| h.parse::<u32>().ok()),
            });
            let (status, v) = to_instance(env, s, "/mandates/propose", &body).await?;
            if status != 200 {
                return Ok((status, v));
            }
            let m: Mandate = serde_json::from_value(v)?;
            let link = offer(env, s, &m).await?;
            Ok((
                202,
                json!({ "mandate": m, "pending": { "operation": "mandate.grant", "link": link } }),
            ))
        }
        "mandate.status" => to_instance(env, s, "/mandates/list", &json!({})).await,
        "mandate.end" => {
            let why = text(args, "why").unwrap_or_else(|| format!("ended by {}", who.subject));
            let body =
                json!({ "id": text(args, "id").and_then(|i| i.parse::<u32>().ok()), "why": why });
            to_instance(env, s, "/mandates/end", &body).await
        }
        other => Ok((404, json!(format!("`{other}` is not a mandate operation")))),
    }
}

/// Mints a page grant for mandate `m`, puts it in the inbox, and answers its link.
async fn offer(env: &Env, s: &Settings, m: &Mandate) -> Result<String> {
    let token = platform::random_hex(32)?;
    let expires = now() + crate::grants::PAGE_TTL;
    let (status, v) = to_instance(
        env,
        s,
        "/mandates/offer",
        &json!({ "id": m.id, "token": token, "expires": expires }),
    )
    .await?;
    if status != 200 {
        return Err(worker::Error::RustError(v.to_string()));
    }
    let path = format!("/mandate/{}?grant={token}", m.id);
    let waiting = Waiting {
        number: m.id,
        operation: "mandate.grant".into(),
        title: m.summary.clone(),
        path: path.clone(),
        expires,
    };
    elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await?;
    let host = env
        .var(cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok(format!("https://{host}{path}"))
}

/// Whether the mandate `change` names approves `operation` on it: the mandate's number, when
/// it does; none, saying why in the log, when the owner is asked as usual.
///
/// # Errors
///
/// When a binding fails.
pub async fn approves(
    env: &Env,
    s: &Settings,
    change: &Change,
    operation: &str,
) -> Result<Option<u32>> {
    if change.mandate.is_none() {
        return Ok(None);
    }
    let (status, v) = to_instance(
        env,
        s,
        "/mandates/decide",
        &json!({ "change": change, "operation": operation }),
    )
    .await?;
    let id = v["id"].as_u64().and_then(|i| u32::try_from(i).ok());
    match (
        status,
        serde_json::from_value::<Decision>(v["decision"].clone()),
    ) {
        (200, Ok(Decision::Allow)) => Ok(id),
        (200, Ok(Decision::Pause(why))) => {
            worker::console_log!("mandate paused: {why}");
            let body = format!("mandate {}: {why}", change.mandate.unwrap_or_default());
            super::push::notify(env, s, "Mandate paused", &body, "/approvals").await;
            Ok(None)
        }
        (200, Ok(Decision::Ask(why))) => {
            worker::console_log!("mandate: the owner approves {operation}: {why}");
            Ok(None)
        }
        _ => {
            worker::console_warn!("mandate: {v}");
            Ok(None)
        }
    }
}

/// Pauses every active mandate, saying `why`.
///
/// # Errors
///
/// When a binding fails.
pub async fn pause_all(env: &Env, s: &Settings, why: &str) -> Result<()> {
    to_instance(env, s, "/mandates/pause", &json!({ "why": why })).await?;
    Ok(())
}

/// The `Instance` Durable Object's side: `/mandates/propose`, `list`, `offer`, `peek`, `grant`,
/// `end`, `decide` and `pause`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Mandates = storage.get(MANDATES).await?.unwrap_or_default();
    let n = |k: &str| body[k].as_u64().and_then(|v| u32::try_from(v).ok());
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let answer: std::result::Result<Value, String> = match path {
        "/mandates/propose" => {
            let scope: Vec<String> = body["scope"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            all.propose(
                &s("adr"),
                &s("summary"),
                scope,
                body["may_deploy"].as_bool() == Some(true),
                n("budget"),
                n("hours"),
            )
            .map(|m| json!(m))
        }
        "/mandates/list" => return Response::from_json(&json!({ "mandates": all.list })),
        "/mandates/offer" => all
            .offer(
                n("id").unwrap_or_default(),
                &s("token"),
                now(),
                body["expires"].as_u64().unwrap_or_default(),
            )
            .map(|()| json!({ "status": "offered" })),
        "/mandates/peek" => {
            return match all.peek(&s("token"), now()) {
                Ok(m) => Response::from_json(m),
                Err(why) => Response::error(why, 403),
            };
        }
        "/mandates/grant" => all.grant(&s("token"), &s("by"), now()).map(|m| json!(m)),
        "/mandates/decide" => {
            let change: Change = match serde_json::from_value(body["change"].clone()) {
                Ok(c) => c,
                Err(e) => return Response::error(format!("unreadable change: {e}"), 400),
            };
            match all.decide(&change, &s("operation"), now()) {
                Some((id, d)) => Ok(json!({ "id": id, "decision": d })),
                None => Err("the change names no mandate".into()),
            }
        }
        "/mandates/pause" => Ok(json!({ "paused": all.pause_all(&s("why")) })),
        "/mandates/end" => match n("id") {
            Some(id) => all.end(id, &s("why")).map(|m| json!(m)),
            None => Err("`id` must be a mandate number".into()),
        },
        _ => return Response::error("not found", 404),
    };
    storage.put(MANDATES, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}

/// The mandate number in `/mandate/<id>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/mandate/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Serves `/mandate/<id>` for `who`: the page, and the POST that grants it with the code.
///
/// # Errors
///
/// When a binding fails.
pub async fn page_serve(
    mut req: Request,
    env: &Env,
    s: &Settings,
    who: &Identity,
) -> Result<Response> {
    if !who.human {
        return page::refused(403, "only a person signed in to Access grants a mandate");
    }
    let Some(id) = number(&req.path()) else {
        return page::refused(404, "not found");
    };
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    match req.method() {
        Method::Get => {
            let Some(token) = url.query().and_then(|q| approve::field(q, "grant")) else {
                return page::refused(400, "this link has no grant");
            };
            let (status, v) =
                to_instance(env, s, "/mandates/peek", &json!({ "token": token })).await?;
            match serde_json::from_value::<Mandate>(v.clone()) {
                Ok(m) if status == 200 && m.id == id => {
                    page::html(200, &approve::mandate_page(&m, &token))
                }
                _ => page::refused(403, v.as_str().unwrap_or("this link is not valid")),
            }
        }
        Method::Post => {
            let origin = req.headers().get("origin")?;
            let site = req.headers().get("sec-fetch-site")?;
            if !approve::same_origin(origin.as_deref(), site.as_deref(), &host) {
                return page::refused(403, "this form is posted only from its own page");
            }
            let form = req.text().await?;
            let Some(token) = approve::field(&form, "grant") else {
                return page::refused(400, "the form is missing its grant");
            };
            let code = approve::field(&form, "code").unwrap_or_default();
            if let Err(why) = elicit::check_code(env, s, code.trim()).await? {
                return page::refused(403, &why);
            }
            let (status, v) = to_instance(
                env,
                s,
                "/mandates/grant",
                &json!({ "token": token, "by": who.subject }),
            )
            .await?;
            let _ = elicit::inbox(
                env,
                s,
                "/approvals/remove",
                &json!({ "path": format!("/mandate/{id}?grant={token}") }),
            )
            .await;
            match serde_json::from_value::<Mandate>(v.clone()) {
                Ok(m) if status == 200 => page::html(
                    200,
                    &approve::outcome(
                        &format!("Mandate {} granted", m.id),
                        &format!(
                            "{}\nActive for {} hours, up to {} merges. Recorded as {}.",
                            m.summary, m.hours, m.budget, who.subject
                        ),
                    ),
                ),
                _ => page::refused(409, v.as_str().unwrap_or("the mandate was not granted")),
            }
        }
        _ => page::refused(405, "method not allowed"),
    }
}
