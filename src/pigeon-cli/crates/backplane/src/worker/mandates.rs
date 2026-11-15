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
use crate::core::approval::windows::{self, BUDGET, Window, Windows};
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
        .var(crate::cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok(format!("https://{host}{path}"))
}

/// What a window or mandate says of an operation, as the Worker acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run it as `window:<id>` or `mandate:<id>`.
    Allow(u32),
    /// Approve it as `window:<id>` or `mandate:<id>` without queuing its merge, which a hold
    /// defers, saying what the merge waits on.
    ApproveHeld(u32, String),
    /// A hold defers it, saying what it waits on; nobody is asked.
    Held(String),
    /// Ask the owner as usual.
    No,
}

/// Whether the mandate `change` names approves `operation` on it; the owner is asked as usual,
/// saying why in the log, when it doesn't.
///
/// # Errors
///
/// When a binding fails.
pub async fn approves(
    env: &Env,
    s: &Settings,
    change: &Change,
    operation: &str,
) -> Result<Verdict> {
    if change.mandate.is_none() {
        return Ok(Verdict::No);
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
        (200, Ok(Decision::Allow)) => Ok(allowed(id, operation, &v)),
        (200, Ok(Decision::Hold(why))) => Ok(Verdict::Held(why)),
        (200, Ok(Decision::Pause(why))) => {
            worker::console_log!("mandate paused: {why}");
            let body = format!("mandate {}: {why}", change.mandate.unwrap_or_default());
            super::push::notify(env, s, "Mandate paused", &body, "/approvals").await;
            Ok(Verdict::No)
        }
        (200, Ok(Decision::Ask(why))) => {
            worker::console_log!("mandate: the owner approves {operation}: {why}");
            Ok(Verdict::No)
        }
        _ => {
            worker::console_warn!("mandate: {v}");
            Ok(Verdict::No)
        }
    }
}

/// An allowed `operation` under `id`: an approval whose merge `decided` says a hold defers is
/// approved without queuing it.
fn allowed(id: Option<u32>, operation: &str, decided: &Value) -> Verdict {
    match (id, decided["held"].as_str()) {
        (None, _) => Verdict::No,
        (Some(id), Some(why)) if operation == "change.approve" => {
            Verdict::ApproveHeld(id, why.to_string())
        }
        (Some(id), _) => Verdict::Allow(id),
    }
}

/// What a hold defers a merge of `change` on under `decided`, if anything.
fn held_merge(decided: crate::mandates::Decision) -> Option<String> {
    match decided {
        Decision::Hold(why) => Some(why),
        _ => None,
    }
}

/// Tells the event log and the owner of `kind` (`auto.held`, `auto.resumed` or `auto.ended`).
async fn tell(env: &Env, s: &Settings, kind: &str, title: &str, said: &str) {
    if let Err(e) = super::events::append(env, s, kind, None, None, said, true).await {
        worker::console_error!("{kind}: {e}");
    }
    super::push::notify(env, s, title, said, "/approvals").await;
}

/// Holds every active window and mandate after the deploy of `sha` failed for `why`; see the
/// iOS approvals ADR's 2026-11-15 amendment.
///
/// # Errors
///
/// When a binding fails.
pub async fn hold_all(env: &Env, s: &Settings, sha: &str, why: &str) -> Result<()> {
    let body = json!({ "sha": sha, "why": why });
    let short = sha.get(..12).unwrap_or(sha);
    let (_, v) = to_instance(env, s, "/windows/hold", &body).await?;
    if let Some(id) = v["window"].as_u64() {
        if v["ended"].as_bool() == Some(true) {
            let said = format!("Auto window {id} ended: the deploy of {short} failed again: {why}");
            tell(env, s, "auto.ended", "Auto ended", &said).await;
        } else {
            let said = format!(
                "Auto window {id} held: the deploy of {short} failed: {why}; it resumes when a deploy passes"
            );
            tell(env, s, "auto.held", "Auto held", &said).await;
        }
    }
    let (_, v) = to_instance(env, s, "/mandates/hold", &body).await?;
    for (key, kind, title, how) in [
        (
            "held",
            "auto.held",
            "Mandate held",
            "held until a deploy passes",
        ),
        (
            "ended",
            "auto.ended",
            "Mandate ended",
            "ended: its hold is spent",
        ),
    ] {
        for id in v[key].as_array().into_iter().flatten() {
            let said = format!("mandate {id} {how}: the deploy of {short} failed: {why}");
            tell(env, s, kind, title, &said).await;
        }
    }
    Ok(())
}

/// Resumes every held window and mandate after the deploy of `sha` passed.
///
/// # Errors
///
/// When a binding fails.
pub async fn resume_all(env: &Env, s: &Settings, sha: &str) -> Result<()> {
    let short = sha.get(..12).unwrap_or(sha);
    let (_, v) = to_instance(env, s, "/windows/resume", &json!({})).await?;
    if let Ok(w) = serde_json::from_value::<Window>(v["window"].clone()) {
        let left = w.expires.saturating_sub(now()) / 60;
        let said = format!(
            "Auto window {} resumed: the deploy of {short} passed; {}h{:02}m and {} merges left",
            w.id,
            left / 60,
            left % 60,
            BUDGET.saturating_sub(w.used)
        );
        tell(env, s, "auto.resumed", "Auto resumed", &said).await;
    }
    let (_, v) = to_instance(env, s, "/mandates/resume", &json!({})).await?;
    for id in v["resumed"].as_array().into_iter().flatten() {
        let said = format!("mandate {id} resumed: the deploy of {short} passed");
        let _ = super::events::append(env, s, "auto.resumed", None, None, &said, true).await;
    }
    Ok(())
}

/// Where the `Instance` keeps the Auto windows.
const WINDOWS: &str = "windows";

/// The Auto window that allows `operation` on `change`, if one is active and does; see
/// `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §5.
///
/// # Errors
///
/// When a binding fails.
pub async fn window_approves(
    env: &Env,
    s: &Settings,
    change: &Change,
    operation: &str,
) -> Result<Option<u32>> {
    Ok(match window_verdict(env, s, change, operation).await? {
        Verdict::Allow(id) => Some(id),
        _ => None,
    })
}

/// What the Auto window says of `operation` on `change`.
///
/// # Errors
///
/// When a binding fails.
pub async fn window_verdict(
    env: &Env,
    s: &Settings,
    change: &Change,
    operation: &str,
) -> Result<Verdict> {
    let (status, v) = to_instance(
        env,
        s,
        "/windows/decide",
        &json!({ "change": change, "operation": operation }),
    )
    .await?;
    let id = v["id"].as_u64().and_then(|i| u32::try_from(i).ok());
    match (
        status,
        serde_json::from_value::<Decision>(v["decision"].clone()),
    ) {
        (200, Ok(Decision::Allow)) => Ok(allowed(id, operation, &v)),
        (200, Ok(Decision::Hold(why))) => Ok(Verdict::Held(why)),
        _ => Ok(Verdict::No),
    }
}

/// `/windows/*` on the `Instance`: `current`, `start`, `end`, `pause`, `hold`, `resume` and
/// `decide`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve_windows(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let all: Windows = storage.get(WINDOWS).await?.unwrap_or_default();
    let t = now();
    let all = all.settle(t);
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let (all, answer): (Windows, std::result::Result<Value, String>) = match path {
        "/windows/current" => {
            let current = all.current().cloned();
            (all, Ok(json!({ "window": current })))
        }
        "/windows/start" => {
            let hours = body["hours"]
                .as_u64()
                .and_then(|h| u32::try_from(h).ok())
                .unwrap_or_default();
            match all.start(hours, &s("by"), t) {
                Ok((next, w)) => (next, Ok(json!({ "window": w }))),
                Err(why) => (all, Err(why)),
            }
        }
        "/windows/end" => match all.end(&s("why")) {
            Ok((next, w)) => (next, Ok(json!({ "window": w }))),
            Err(why) => (all, Err(why)),
        },
        "/windows/pause" => {
            let (next, paused) = all.pause_all(&s("why"));
            (next, Ok(json!({ "paused": paused })))
        }
        "/windows/hold" => {
            let (next, held) = all.hold_all(&s("sha"), &s("why"), t);
            let (id, ended) = held.unzip();
            (next, Ok(json!({ "window": id, "ended": ended })))
        }
        "/windows/resume" => {
            let (next, resumed) = all.resume_all(t);
            (next, Ok(json!({ "window": resumed })))
        }
        "/windows/decide" => {
            let change: Change = match serde_json::from_value(body["change"].clone()) {
                Ok(c) => c,
                Err(e) => return Response::error(format!("unreadable change: {e}"), 400),
            };
            let held = all
                .current()
                .and_then(|w| held_merge(windows::decide(w, &change, "change.merge", t)));
            match all.decide(&change, &s("operation"), t) {
                (next, Some((id, d))) => {
                    (next, Ok(json!({ "id": id, "decision": d, "held": held })))
                }
                (next, None) => (next, Err("no Auto window is active".into())),
            }
        }
        _ => return Response::error("not found", 404),
    };
    storage.put(WINDOWS, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}

/// The `Instance` Durable Object's side: `/mandates/propose`, `list`, `offer`, `peek`, `grant`,
/// `end`, `decide`, `pause`, `hold` and `resume`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let all: Mandates = storage.get(MANDATES).await?.unwrap_or_default();
    let (all, _) = all.settle(now());
    let n = |k: &str| body[k].as_u64().and_then(|v| u32::try_from(v).ok());
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let (all, answer): (Mandates, std::result::Result<Value, String>) = match path {
        "/mandates/propose" => {
            let scope: Vec<String> = body["scope"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            let (all, proposed) = all.propose(
                &s("adr"),
                &s("summary"),
                scope,
                body["may_deploy"].as_bool() == Some(true),
                n("budget"),
                n("hours"),
            );
            (all, proposed.map(|m| json!(m)))
        }
        "/mandates/list" => return Response::from_json(&json!({ "mandates": all.list })),
        "/mandates/offer" => {
            let (all, offered) = all.offer(
                n("id").unwrap_or_default(),
                &s("token"),
                now(),
                body["expires"].as_u64().unwrap_or_default(),
            );
            (all, offered.map(|()| json!({ "status": "offered" })))
        }
        "/mandates/peek" => {
            return match all.peek(&s("token"), now()) {
                Ok(m) => Response::from_json(m),
                Err(why) => Response::error(why, 403),
            };
        }
        "/mandates/grant" => {
            let (all, granted) = all.grant(&s("token"), &s("by"), now());
            (all, granted.map(|m| json!(m)))
        }
        "/mandates/decide" => {
            let change: Change = match serde_json::from_value(body["change"].clone()) {
                Ok(c) => c,
                Err(e) => return Response::error(format!("unreadable change: {e}"), 400),
            };
            let held = change.mandate.and_then(|id| all.get(id)).and_then(|m| {
                held_merge(crate::mandates::decide(m, &change, "change.merge", now()))
            });
            match all.decide(&change, &s("operation"), now()) {
                (all, Some((id, d))) => (all, Ok(json!({ "id": id, "decision": d, "held": held }))),
                (all, None) => (all, Err("the change names no mandate".into())),
            }
        }
        "/mandates/pause" => {
            let (all, paused) = all.pause_all(&s("why"));
            (all, Ok(json!({ "paused": paused })))
        }
        "/mandates/hold" => {
            let (all, held, ended) = all.hold_all(&s("sha"), &s("why"), now());
            (all, Ok(json!({ "held": held, "ended": ended })))
        }
        "/mandates/resume" => {
            let (all, resumed) = all.resume_all(now());
            (all, Ok(json!({ "resumed": resumed })))
        }
        "/mandates/end" => match n("id") {
            Some(id) => {
                let (all, ended) = all.end(id, &s("why"));
                (all, ended.map(|m| json!(m)))
            }
            None => (all, Err("`id` must be a mandate number".into())),
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
                    page::html_with(200, &approve::mandate_page(&m, &token), approve::PROOF_CSP)
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
            let purpose = crate::passkeys::grant_purpose(&token);
            let shown = format!("/mandate/{id}?grant={token}");
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) = to_instance(
                env,
                s,
                "/mandates/grant",
                &json!({ "token": token, "by": super::account::approver(env, s).await? }),
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

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix("/mandate/"),
    serve_route,
)];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
}
