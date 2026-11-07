//! The control plane's Worker (control-plane ADR §1, §3, §9, §10): every request is verified
//! with Access, then served as MCP on `/mcp` or the wire form on `/v0/`; changes live in the
//! `ChangeRun` Durable Object, their jobs in `Checker` and `Runner` Containers, and Artifacts push events
//! arrive on a queue; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.
#![allow(
    missing_docs,
    reason = "`durable_object` generates undocumented wasm-bindgen items"
)]

mod change;
mod checker;
mod elicit;
mod gitdoor;
mod main_run;
mod platform;

use serde_json::{Value, json};
use worker::{
    DurableObject, Env, MessageBatch, MessageExt, Method, Request, Response, Result, State,
    durable_object, event,
};

pub use change::ChangeRun;
pub use checker::{Checker, Runner};
pub use main_run::MainRun;
use platform::{Settings, json_request, now};

use crate::access::{self, Identity};
use crate::changes::{Change, Event};
use crate::control::{self, Routed};
use crate::grants;
use crate::mcp::{self, Action};
use crate::runs::{Lost, Queue, Run};
use crate::wire::{self, Response as WireResponse};

/// The storage key holding an instance's [`Queue`].
const QUEUE: &str = "queue";

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    let settings = Settings::from(&env)?;
    let who = match platform::identify(&req, &settings).await {
        Ok(who) => who,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    if req.path().starts_with("/git/") {
        return gitdoor::serve(req, &env, &settings, &who).await;
    }
    match (req.method(), req.path().as_str()) {
        (Method::Post, "/mcp") => {
            let key = platform::secret(&env, "SESSION_KEY").ok();
            let session = req
                .headers()
                .get(mcp::SESSION_HEADER)?
                .zip(key.as_deref())
                .and_then(|(id, k)| mcp::session(&id, k.as_bytes()))
                .unwrap_or_default();
            let message: Value = match req.json().await {
                Ok(v) => v,
                Err(_) => {
                    return Response::from_json(&mcp::error(
                        &Value::Null,
                        mcp::code::INVALID_REQUEST,
                        "not JSON",
                    ));
                }
            };
            match mcp::parse(&message, crate::VERSION) {
                Action::Reply(v) => Response::from_json(&v),
                Action::Initialize { reply, session } => {
                    let mut r = Response::from_json(&reply)?;
                    if let Some(k) = key {
                        r.headers_mut().set(
                            mcp::SESSION_HEADER,
                            &mcp::session_id(&session, k.as_bytes()),
                        )?;
                    }
                    Ok(r)
                }
                Action::Accept => Ok(Response::empty()?.with_status(202)),
                Action::Answer { id, result } => elicit::answer(&env, &id, &result).await,
                Action::Call { id, request } => {
                    if !who.human && grants::OPERATIONS.contains(&request.operation.as_str()) {
                        return offer(&env, &session, &id, &request).await;
                    }
                    let outcome = run(&env, &settings, &who, &request).await;
                    Response::from_json(&mcp::answer(&id, outcome))
                }
            }
        }
        (Method::Get, "/v0/operations") => Response::from_json(&wire::catalog()),
        (Method::Post, "/v0/run") => {
            let request: wire::Request = req.json().await?;
            match run(&env, &settings, &who, &request).await {
                Ok(r) => Response::from_json(&r),
                Err(e) => Response::error(e.to_string(), 400),
            }
        }
        _ => Response::error("not found", 404),
    }
}

/// A service token's call of an owner's operation: a card asking for the owner's code, when
/// the client shows forms.
async fn offer(
    env: &Env,
    session: &mcp::Session,
    id: &Value,
    request: &wire::Request,
) -> Result<Response> {
    let refused =
        |why: String| Response::from_json(&mcp::answer(id, Err(crate::Error::Invalid(vec![why]))));
    let number = request.args.get("number").and_then(|v| match v {
        Value::String(s) => s.parse().ok(),
        other => other.as_u64().and_then(|n| u32::try_from(n).ok()),
    });
    let Some(number) = number else {
        return refused("`number` must be a change number".into());
    };
    if session.form_elicitation {
        return match elicit::card(env.clone(), id.clone(), number, request.operation.clone()).await
        {
            Ok(r) => Ok(r),
            Err(why) => refused(why),
        };
    }
    refused(
        "this client doesn't show forms, so it can't ask for the owner's code; approve with cloudflared, or reconnect a client that shows forms".into(),
    )
}

fn change_stub(env: &Env, repo: &str, number: u32) -> Result<worker::Stub> {
    env.durable_object("CHANGE_RUN")?
        .id_from_name(&format!("{repo}#{number}"))?
        .get_stub()
}

/// Sends `body` to change `number`'s Durable Object at `path`: its status and JSON answer.
async fn to_change(
    env: &Env,
    repo: &str,
    number: u32,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value)> {
    let req = json_request(&format!("https://change{path}"), method, body, None)?;
    let mut r = change_stub(env, repo, number)?
        .fetch_with_request(req)
        .await?;
    let status = r.status_code();
    let text = r.text().await?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    ))
}

fn respond(operation: &str, status: u16, result: Value) -> WireResponse {
    let ok = (200..300).contains(&status);
    let violations = if ok {
        result["change"]["violations"]
            .as_array()
            .or_else(|| result["violations"].as_array())
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![
            result
                .as_str()
                .map_or_else(|| result.to_string(), str::to_string),
        ]
    };
    WireResponse {
        operation: operation.to_string(),
        ok: ok && violations.is_empty(),
        result,
        violations,
    }
}

/// Runs one operation as `who`: validated, authorized, routed and audited.
async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    request: &wire::Request,
) -> std::result::Result<WireResponse, crate::Error> {
    let op = wire::validate(request)?;
    access::authorize(op, who).map_err(|m| crate::Error::Invalid(vec![m]))?;
    if !op.control_plane() {
        return Err(crate::Error::Invalid(vec![format!(
            "`{}` runs in an instance's Container, which the control plane doesn't dispatch yet",
            op.name
        )]));
    }
    let mut request = request.clone();
    if op.name == "change.approve" && !request.args.contains_key("sha") {
        let number = control::route(
            &wire::Request {
                operation: "change.status".into(),
                args: request.args.clone(),
            },
            &who.subject,
            now(),
        )
        .ok()
        .and_then(|r| match r {
            Routed::Status { number } => Some(number),
            _ => None,
        })
        .ok_or_else(|| crate::Error::Invalid(vec!["`number` must be a change number".into()]))?;
        let (_, v) = to_change(env, &s.repo, number, Method::Get, "/status", None)
            .await
            .map_err(|e| crate::Error::Invalid(vec![e.to_string()]))?;
        let head = v["change"]["head"]
            .as_str()
            .ok_or_else(|| crate::Error::Invalid(vec![format!("change {number} has no head")]))?;
        request.args.insert("sha".into(), Value::from(head));
    }
    let request = &request;
    let routed =
        control::route(request, &who.subject, now()).map_err(|m| crate::Error::Invalid(vec![m]))?;
    let outcome = dispatch(env, s, who, op.name, routed).await;
    let (target, result) = match &outcome {
        Ok((t, r)) => (t.clone(), if r.ok { "ok" } else { "refused" }),
        Err(_) => (None, "error"),
    };
    let audit = control::audit(
        now(),
        &who.subject,
        op.name,
        &s.instance,
        target.as_deref(),
        result,
    );
    if let Err(e) = platform::d1(env, &[control::instance_row(&s.instance, now()), audit]).await {
        worker::console_error!("audit: {e}");
    }
    outcome
        .map(|(_, r)| r)
        .map_err(|e| crate::Error::Invalid(vec![e.to_string()]))
}

async fn dispatch(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    routed: Routed,
) -> Result<(Option<String>, WireResponse)> {
    match routed {
        Routed::Propose {
            slug,
            title,
            plan,
            scope,
        } => {
            for _ in 0..3 {
                let rows =
                    platform::d1_rows(env, &control::next_number(&s.instance, &s.repo, s.floor))
                        .await?;
                let number = rows
                    .first()
                    .and_then(|r| r["n"].as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(s.floor);
                let change =
                    Change::propose(number, &slug, &title, &plan, scope.clone(), &who.subject);
                let body = serde_json::to_value(&change)?;
                let (status, v) =
                    to_change(env, &s.repo, number, Method::Post, "/propose", Some(&body)).await?;
                if status != 409 {
                    let result =
                        json!({ "number": number, "branch": change.branch(), "change": v });
                    return Ok((Some(number.to_string()), respond(operation, status, result)));
                }
            }
            Ok((
                None,
                respond(
                    operation,
                    409,
                    json!("no change number was free; try again"),
                ),
            ))
        }
        Routed::Event { number, event } => {
            let body = serde_json::to_value(&event)?;
            let (status, v) =
                to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await?;
            Ok((
                Some(number.to_string()),
                respond(operation, status, json!({ "change": v })),
            ))
        }
        Routed::Status { number } => {
            let (status, v) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
            Ok((Some(number.to_string()), respond(operation, status, v)))
        }
        Routed::DeployStatus => {
            let mut r = main_stub(env, &s.repo)?
                .fetch_with_str("https://main/status")
                .await?;
            let v: Value = r.json().await?;
            Ok((None, respond(operation, r.status_code(), v)))
        }
        Routed::GitTokens => {
            let api = platform::secret(env, "ARTIFACTS_TOKEN")?;
            let account = platform::account(&api).await?;
            let path = format!(
                "/accounts/{account}/artifacts/namespaces/{}/repos/{}/tokens?state=all&per_page=100",
                s.namespace, s.repo
            );
            let v = platform::cloudflare(&api, Method::Get, &path, None).await?;
            let tokens = control::token_rows(&v);
            Ok((None, respond(operation, 200, json!({ "tokens": tokens }))))
        }
        Routed::List { state } => {
            let rows =
                platform::d1_rows(env, &control::list(&s.instance, &s.repo, state.as_deref()))
                    .await?;
            Ok((None, respond(operation, 200, json!({ "changes": rows }))))
        }
    }
}

fn main_stub(env: &Env, repo: &str) -> Result<worker::Stub> {
    env.durable_object("MAIN_RUN")?
        .id_from_name(&format!("{repo}#main"))?
        .get_stub()
}

/// Artifacts push events: a push to a change's branch is that change's `Pushed`; a push to
/// `main` goes to its pipeline.
#[event(queue)]
async fn queue(batch: MessageBatch<Value>, env: Env, _ctx: worker::Context) -> Result<()> {
    let s = Settings::from(&env)?;
    for message in batch.messages()? {
        let Some(push) = control::push(message.body()) else {
            message.ack();
            continue;
        };
        if push.repo.as_deref().is_some_and(|r| r != s.repo) {
            message.ack();
            continue;
        }
        let number = match push.target {
            control::Target::Main => {
                let body = json!({ "sha": push.sha });
                let req = json_request("https://main/pushed", Method::Post, Some(&body), None)?;
                match main_stub(&env, &s.repo)?.fetch_with_request(req).await {
                    Ok(r) if (200..300).contains(&r.status_code()) => message.ack(),
                    _ => message.retry(),
                }
                continue;
            }
            control::Target::Change(n) => n,
        };
        let body = serde_json::to_value(Event::Pushed {
            sha: push.sha.clone(),
        })?;
        match to_change(&env, &s.repo, number, Method::Post, "/event", Some(&body)).await {
            Ok((status, v)) if (200..300).contains(&status) || status == 409 || status == 404 => {
                if status != 200 {
                    worker::console_warn!("push to change {number} refused: {v}");
                }
                message.ack();
            }
            _ => message.retry(),
        }
    }
    Ok(())
}

/// One instance's runs, one at a time (ADR §3); reached only by the Worker.
#[durable_object]
pub struct Instance {
    state: State,
    env: Env,
}

impl DurableObject for Instance {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        if path.starts_with("/elicit/") {
            let body: Value = req.json().await?;
            return elicit::serve(&self.state, &self.env, &path, body).await;
        }
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        let method = req.method();
        // Only storage is awaited between the queue's read and write (ADR §3).
        let submitted: Option<Run> = match (&method, segments.as_slice()) {
            (Method::Post, ["runs"]) => Some(req.json().await?),
            _ => None,
        };
        let storage = self.state.storage();
        let mut queue: Queue = storage.get(QUEUE).await?.unwrap_or_default();
        let answer = match (method, segments.as_slice(), submitted) {
            (Method::Get, ["runs"], _) => return Response::from_json(&queue),
            (Method::Post, ["runs"], Some(run)) => match queue.enqueue(run) {
                Ok(()) => Response::from_json(&queue),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", "start"], _) => Response::from_json(&queue.start().cloned()),
            (Method::Post, ["runs", id, "finish"], _) => match queue.finish(id) {
                Ok(run) => Response::from_json(&run),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", id, "lost"], _) => match queue.lost(id) {
                Ok(Lost::Retry(run)) => Response::from_json(&serde_json::json!({ "retry": run })),
                Ok(Lost::Failed(run)) => Response::from_json(&serde_json::json!({ "failed": run })),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            _ => return Response::error("not found", 404),
        };
        storage.put(QUEUE, &queue).await?;
        answer
    }
}
