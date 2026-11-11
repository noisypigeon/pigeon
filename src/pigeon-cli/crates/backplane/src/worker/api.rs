//! The API surface: MCP on `/mcp`, and the wire form on `/v0/`; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.

use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use worker::{Env, Method, Request, Response, Result};

use super::platform::{self, Settings};
use super::{Route, elicit, events, offer, run, wait};
use crate::access::Identity;
use crate::control;
use crate::grants;
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
    let key = platform::secret(env, "SESSION_KEY").ok();
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
        Action::Answer { id, result } => elicit::answer(env, &id, &result).await,
        Action::Call { id, request } => {
            let request = control::retarget(&request);
            if request.operation == "approval.wait" {
                return wait(env, &session, &id, &request).await;
            }
            if !who.human && grants::OPERATIONS.contains(&request.operation.as_str()) {
                return offer(env, settings, &id, &request).await;
            }
            let outcome = run(env, settings, who, &request).await;
            Response::from_json(&mcp::answer(&id, outcome))
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
