//! The approvals inbox and the owner's code check, on the `Instance` Durable Object. The form
//! card is gone; see `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §2.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::platform::{self, Settings, json_request, now};
use crate::totp::{self, Guard};

/// The storage key of the code's [`Guard`].
pub(super) const GUARD: &str = "totp";

/// The storage key of the inbox: every approval waiting for the page.
const WAITING: &str = "waiting";

async fn to_instance(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<Value> {
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
    let text = r.text().await?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// A client's answer to a card from before the form card was removed: accepted and dropped.
///
/// # Errors
///
/// Never; it answers `202 Accepted`.
#[expect(clippy::unused_async, reason = "the MCP route awaits it")]
pub async fn answer(_env: &Env, key: &str, _result: &Value) -> Result<Response> {
    worker::console_log!("card {key}: no card is waiting; approvals are made on the page");
    Ok(Response::empty()?.with_status(202))
}

/// The `Instance` Durable Object's side: the inbox's `/approvals/*` and `/code/verify`.
///
/// # Errors
///
/// When storage or a binding fails.
pub async fn serve(state: &State, env: &Env, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    match path {
        "/approvals/add" => {
            let w: crate::approve::Waiting = serde_json::from_value(body)?;
            let mut list: Vec<crate::approve::Waiting> =
                storage.get(WAITING).await?.unwrap_or_default();
            list.retain(|x| x.expires > now() && x.path != w.path);
            list.push(w);
            storage.put(WAITING, &list).await?;
            Response::from_json(&json!({ "status": "added" }))
        }
        "/approvals/list" => {
            let mut list: Vec<crate::approve::Waiting> =
                storage.get(WAITING).await?.unwrap_or_default();
            list.retain(|x| x.expires > now());
            Response::from_json(&list)
        }
        "/approvals/remove" => {
            let path = body["path"].as_str().unwrap_or_default();
            let mut list: Vec<crate::approve::Waiting> =
                storage.get(WAITING).await?.unwrap_or_default();
            list.retain(|x| x.expires > now() && x.path != path);
            storage.put(WAITING, &list).await?;
            Response::from_json(&json!({ "status": "removed" }))
        }
        "/code/verify" => {
            let code = body["code"].as_str().unwrap_or_default();
            match verify(state, env, code).await? {
                Ok(()) => Response::from_json(&json!({ "status": "ok" })),
                Err(why) => Response::from_json(&json!({ "status": "refused", "why": why })),
            }
        }
        _ => Response::error("not found", 404),
    }
}

/// Checks the owner's `code` against the authenticator secret, under the one guard the card and
/// the page share: each step used once, and five wrong codes in an hour lock it.
async fn verify(state: &State, env: &Env, code: &str) -> Result<std::result::Result<(), String>> {
    let secret = totp::base32_decode(&platform::secret(env, "APPROVER_TOTP")?)?;
    let storage = state.storage();
    let guard: Guard = storage.get(GUARD).await?.unwrap_or_default();
    let (guard, checked) = guard.verify(&secret, code, now());
    storage.put(GUARD, &guard).await?;
    Ok(checked.map(|_| ()))
}

/// Checks `code` with the `Instance` Durable Object.
///
/// # Errors
///
/// When the binding fails.
pub async fn check_code(
    env: &Env,
    s: &Settings,
    code: &str,
) -> Result<std::result::Result<(), String>> {
    let v = to_instance(env, s, "/code/verify", &json!({ "code": code })).await?;
    Ok(if v["status"].as_str() == Some("ok") {
        Ok(())
    } else {
        Err(v["why"]
            .as_str()
            .unwrap_or("the code was refused")
            .to_string())
    })
}

/// Lists, adds or removes inbox entries with the `Instance` Durable Object, at `path`.
///
/// # Errors
///
/// When the binding fails.
pub async fn inbox(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<Value> {
    to_instance(env, s, path, body).await
}
