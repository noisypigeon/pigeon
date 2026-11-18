//! The API surface: MCP on `/mcp`; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.

use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result};

use super::platform::{self, Settings};
use super::{Route, elicit, offer, run, wait};
use crate::access::Identity;
use crate::control;
use crate::core::app::devices::Device;
use crate::core::approval::grants;
use crate::mcp::{self, Action};
use crate::routes::Path;
use crate::wire::{self, Response as WireResponse};

/// The routes this surface serves.
pub(super) const ROUTES: &[Route] = &[Route::only(Method::Post, Path::Exact("/mcp"), mcp_route)];

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
        device: None,
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
    /// The device whose signature `/v0/app/mcp` verified; `None` on `/mcp`.
    pub device: Option<&'a Device>,
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
                device: who.device,
            };
            super::passkeys::proof_submit(env, settings, &person, &args, door.host).await?
        };
        let title = tool.replace('_', ".");
        return Response::from_json(&mcp::proof_answer(&id, &title, outcome));
    }
    match action {
        Action::Reply(v) => {
            let listing = message.get("method").and_then(Value::as_str) == Some("tools/list");
            let kind = if door.device.is_some() {
                wire::Door::Device
            } else {
                wire::Door::Access
            };
            let v = if listing { mcp::for_door(v, kind) } else { v };
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
            if wire::DEVICE_OPS.contains(&request.operation.as_str()) {
                let outcome = match door.device {
                    None => Err(crate::Error::Invalid(vec![format!(
                        "`{}` runs only on the app's door, /v0/app/mcp",
                        request.operation
                    )])),
                    Some(device) => Ok(device_op(env, settings, device, &request).await?),
                };
                return Response::from_json(&mcp::answer(&id, outcome));
            }
            if request.operation == "push.subscribe" && !door.person {
                let why = "`push.subscribe` is a person's, signed in".to_string();
                let refused = Err(crate::Error::Invalid(vec![why]));
                return Response::from_json(&mcp::answer(&id, refused));
            }
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

/// A device-door operation, run as `device`; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its R8 amendment.
#[expect(clippy::too_many_lines, reason = "one arm per device operation")]
async fn device_op(
    env: &Env,
    s: &Settings,
    device: &Device,
    request: &wire::Request,
) -> Result<WireResponse> {
    let arg = |k: &str| {
        request
            .args
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let op = request.operation.as_str();
    let reply = |ok: bool, result: Value, violations: Vec<String>| WireResponse {
        operation: op.into(),
        ok,
        result,
        violations,
    };
    let route = |method: Method, path: &'static str, body: Value| async move {
        super::account::route(env, s, device, &method, path, &body).await
    };
    let response = match op {
        "device.show" => {
            return Ok(reply(
                true,
                json!({"device": device.id, "name": device.name}),
                vec![],
            ));
        }
        "avatar.edit" => return Ok(reply(true, json!({}), vec![])),
        "device.push" => {
            if !crate::core::app::push::token_ok(&arg("token")) {
                return Ok(reply(
                    false,
                    Value::Null,
                    vec!["not an APNs device token".into()],
                ));
            }
            let body = json!({"id": device.id, "token": arg("token")});
            super::mandates::to_instance(env, s, "/devices/push", &body).await?;
            return Ok(reply(true, json!({"status": "ok"}), vec![]));
        }
        "instances.list" => super::instances::list(env, s, device).await?,
        "members.list" => super::members::route(env, s, device).await?,
        "profile.show" | "profile.edit" => {
            route(Method::Get, "/v0/app/profile", Value::Null).await?
        }
        "profile.update" => {
            let mut body = Map::new();
            for k in ["name", "appearance"] {
                if let Some(v) = request.args.get(k) {
                    body.insert(k.into(), v.clone());
                }
            }
            route(Method::Patch, "/v0/app/profile", Value::Object(body)).await?
        }
        "avatar.show" => {
            let v = super::elicit::inbox(
                env,
                s,
                "/accounts/avatar/get",
                &json!({"device": device.id}),
            )
            .await?;
            return Ok(match v["error"].as_str() {
                Some(why) => reply(false, Value::Null, vec![why.into()]),
                None => reply(true, v, vec![]),
            });
        }
        "avatar.put" => {
            route(
                Method::Put,
                "/v0/app/profile/avatar",
                json!({"jpeg": arg("jpeg")}),
            )
            .await?
        }
        "avatar.remove" => route(Method::Delete, "/v0/app/profile/avatar", Value::Null).await?,
        "instances.standup" => {
            let body = json!({
                "name": arg("name"),
                "label": arg("label"),
                "token": arg("token"),
                "passkey": {
                    "id": arg("credential"),
                    "client": arg("client_data"),
                    "authenticator": arg("authenticator_data"),
                    "signature": arg("signature"),
                },
            });
            super::instances::standup(env, s, device, &body).await?
        }
        "device.unlink" | "passkey.remove" => {
            let proof = json!({"passkey": {
                "id": arg("credential"),
                "client": arg("client_data"),
                "authenticator": arg("authenticator_data"),
                "signature": arg("signature"),
            }});
            let prefix = if op == "device.unlink" {
                "/v0/app/devices/"
            } else {
                "/v0/app/passkeys/"
            };
            super::account::route(
                env,
                s,
                device,
                &Method::Delete,
                &format!("{prefix}{}", arg("id")),
                &proof,
            )
            .await?
        }
        other => {
            return Ok(reply(
                false,
                Value::Null,
                vec![format!("`{other}` has no device handler")],
            ));
        }
    };
    let mut response = response;
    let status = response.status_code();
    let text = response.text().await?;
    if (200..300).contains(&status) {
        let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
        Ok(reply(true, body, vec![]))
    } else {
        Ok(reply(false, Value::Null, vec![text]))
    }
}
