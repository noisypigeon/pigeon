//! The API surface: MCP on `/mcp`, and the wire form on `/v0/`; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.

use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use worker::{Env, Method, Request, Response, Result};

use super::platform::{self, Settings};
use super::{Route, elicit, events, offer, run, wait};
use crate::access::Identity;
use crate::control;
use crate::core::approval::grants;
use crate::mcp::{self, Action};
use crate::routes::Path;
use crate::wire;

/// The routes this surface serves.
pub(super) const ROUTES: &[Route] = &[
    Route::only(Method::Post, Path::Exact("/mcp"), mcp_route),
    Route::only(Method::Get, Path::Exact("/v0/operations"), operations),
    Route::only(Method::Get, Path::Exact("/v0/events"), events_route),
    Route::only(Method::Post, Path::Exact("/v0/run"), run_route),
];

fn mcp_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> LocalBoxFuture<'a, Result<Response>> {
    Box::pin(serve_mcp(req, env, s, who))
}

async fn serve_mcp(
    mut req: Request,
    env: &Env,
    settings: &Settings,
    who: &Identity,
) -> Result<Response> {
    let sid = req.headers().get(mcp::SESSION_HEADER)?;
    let host = req.url()?.host_str().unwrap_or_default().to_string();
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
    let door = Door {
        name: "access",
        person: who.human,
        host: &host,
    };
    serve_message(env, settings, who, &door, sid.as_deref(), &message).await
}

/// The device-signed door's name.
pub(super) const DEVICE: &str = "device";

/// Which door a message came in by: its name, whether its caller is a person (an Access
/// sign-in, or a linked device), and the host it was sent to; see the 2026-11-17 amendment
/// "two doors, one handler" to `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`.
pub(super) struct Door<'a> {
    /// `access` for `/mcp`, `device` for `/v0/app/mcp`.
    pub name: &'static str,
    /// Whether the proof tools are listed and callable.
    pub person: bool,
    /// The host the message was sent to.
    pub host: &'a str,
}

/// One MCP message, from either door, for `who`.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn serve_message(
    env: &Env,
    settings: &Settings,
    who: &Identity,
    door: &Door<'_>,
    sid: Option<&str>,
    message: &Value,
) -> Result<Response> {
    let key = platform::secret(env, "SESSION_KEY").ok();
    let action = mcp::parse(message, crate::VERSION);
    if let Some(k) = &key {
        let admitted = matches!(action, Action::Initialize { .. })
            || mcp::admits(sid, k.as_bytes(), door.name, &who.subject);
        if !admitted {
            return Response::error("unknown MCP session; initialize again", 404);
        }
    }
    if let Some((id, tool, args)) = mcp::proof_call(message) {
        if !door.person {
            let why = "the proof tools are a person's, signed in or on a linked device";
            return Response::from_json(&mcp::proof_answer(&id, tool, Err(why.into())));
        }
        let outcome = if tool == mcp::PROOF_TOOLS[0] || tool == mcp::PROOF_TOOLS[2] {
            super::passkeys::proof_challenge(env, settings, &args, door.host).await?
        } else {
            let person = Identity {
                subject: who.subject.clone(),
                human: true,
            };
            super::passkeys::proof_submit(env, settings, &person, &args, door.host).await?
        };
        let title = tool.replace('_', ".");
        return Response::from_json(&mcp::proof_answer(&id, &title, outcome));
    }
    match action {
        Action::Reply(v) => {
            let listing = message.get("method").and_then(Value::as_str) == Some("tools/list");
            if listing && door.person {
                return Response::from_json(&mcp::with_proof_tools(v));
            }
            Response::from_json(&v)
        }
        Action::Initialize { reply, mut session } => {
            session.door = door.name.into();
            session.caller = mcp::caller_tag(&who.subject);
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
        Action::Answer { id, result } => elicit::answer(env, &id, &result).await,
        Action::Call { id, request } => {
            let request = control::retarget(&request);
            if request.operation == "approval.wait" {
                return wait(env, &id, &request).await;
            }
            let owners = grants::OPERATIONS.contains(&request.operation.as_str());
            if !who.human && owners {
                return offer(env, settings, &id, &request).await;
            }
            // After the cutover a person's direct call waits for a passkey too; see
            // `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §2.
            // A device's call always waits for the owner's proof, whatever the cutover.
            let waits = owners
                && (door.name == DEVICE || super::passkeys::cutover_made(env, settings).await?);
            if waits {
                let Some(number) = super::number_arg(&request) else {
                    let why = "`number` must be a change number".to_string();
                    let refused = Err(crate::Error::Invalid(vec![why]));
                    return Response::from_json(&mcp::answer(&id, refused));
                };
                let answer = super::ask(env, settings, number, &request).await?;
                return Response::from_json(&mcp::answer(&id, answer));
            }
            let outcome = run(env, settings, who, &request).await;
            Response::from_json(&mcp::answer(&id, outcome))
        }
        Action::Read { id, uri, request } => {
            let outcome = run(env, settings, who, &request).await;
            Response::from_json(&mcp::contents(&id, &uri, outcome))
        }
    }
}

fn operations<'a>(
    _req: Request,
    _env: &'a Env,
    _s: &'a Settings,
    _who: &'a Identity,
) -> LocalBoxFuture<'a, Result<Response>> {
    Box::pin(async { Response::from_json(&wire::catalog()) })
}

fn events_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> LocalBoxFuture<'a, Result<Response>> {
    Box::pin(async move {
        let url = req.url()?;
        let args: serde_json::Map<String, Value> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
            .collect();
        Response::from_json(&events::wait(env, s, &args, who.human).await?)
    })
}

fn run_route<'a>(
    mut req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> LocalBoxFuture<'a, Result<Response>> {
    Box::pin(async move {
        let request: wire::Request = req.json().await?;
        match run(env, s, who, &request).await {
            Ok(r) => Response::from_json(&r),
            Err(e) => Response::error(e.to_string(), 400),
        }
    })
}
