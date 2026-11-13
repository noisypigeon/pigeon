//! The app's approvals: listed, shown, and approved with the owner's passkey, as the approval
//! page does; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §3-§5.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Response, Result};

use super::approve::live;
use super::platform::{Settings, secret};
use super::{elicit, run, to_change};
use crate::access::Identity;
use crate::changes::Change;
use crate::core::app::approvals;
use crate::core::approval::windows;
use crate::grants::Grant;
use crate::passkeys;
use crate::wire;

/// The relying party the app's passkey answers for; its client data's origin is this host.
fn relying_party(env: &Env) -> String {
    env.var("PASSKEY_RP_ID")
        .map_or_else(|_| "platform.pigeon.dev".into(), |v| v.to_string())
}

/// The waiting change approvals, as the app lists them.
///
/// # Errors
///
/// When a binding fails.
pub async fn list(env: &Env, s: &Settings) -> Result<Value> {
    Ok(approvals::list(&live(env, s).await?))
}

/// `GET /v0/app/approvals/<id>`.
///
/// # Errors
///
/// When a binding fails.
pub async fn detail(env: &Env, s: &Settings, id: &str) -> Result<Response> {
    let waiting = live(env, s).await?;
    let Some((w, token)) = approvals::find(&waiting, id) else {
        return Response::error("no such approval waiting", 404);
    };
    let (status, v) = to_change(
        env,
        &s.repo,
        w.number,
        Method::Post,
        "/peek",
        Some(&json!({ "token": token })),
    )
    .await?;
    let (Ok(change), Ok(grant)) = (
        serde_json::from_value::<Change>(v["change"].clone()),
        serde_json::from_value::<Grant>(v["grant"].clone()),
    ) else {
        return Response::error(
            "the approval can't be read",
            if status == 200 { 500 } else { 404 },
        );
    };
    Response::from_json(&approvals::detail(w, &change, &grant))
}

/// `POST /v0/app/challenge` with `{id}`: a passkey challenge for that approval.
///
/// # Errors
///
/// When a binding fails.
pub async fn challenge(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let purpose = match body["op"].as_str() {
        Some("auto") => windows::START.to_string(),
        Some("auto-end") => windows::END.to_string(),
        _ => {
            let id = body["id"].as_str().unwrap_or_default();
            let waiting = live(env, s).await?;
            let Some((_, token)) = approvals::find(&waiting, id) else {
                return Response::error("no such approval waiting", 404);
            };
            passkeys::grant_purpose(token)
        }
    };
    let mut v = elicit::inbox(
        env,
        s,
        "/passkeys/challenge",
        &json!({ "purpose": purpose }),
    )
    .await?;
    v["rp"] = json!(relying_party(env));
    Response::from_json(&v)
}

/// `POST /v0/app/approvals/<id>` with `{passkey: {id, client, authenticator, signature}, hold}`:
/// the passkey checked, the grant redeemed as the owner, and its operation run.
///
/// # Errors
///
/// When a binding fails.
pub async fn approve(env: &Env, s: &Settings, id: &str, body: &Value) -> Result<Response> {
    let waiting = live(env, s).await?;
    let Some((entry, token)) = approvals::find(&waiting, id) else {
        return Response::error("no such approval waiting", 404);
    };
    let (number, token) = (entry.number, token.to_string());
    let rp = relying_party(env);
    let form = approvals::proof_form(body);
    let purpose = passkeys::grant_purpose(&token);
    if let Err(why) = super::passkeys::prove(env, s, &form, &purpose, &rp).await? {
        return Response::error(format!("refused: {why}"), 403);
    }
    let answers = if entry.operation == "change.authorize" {
        let (_, st) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
        let Ok(change) = serde_json::from_value::<Change>(st["change"].clone()) else {
            return Response::error("the change was unreadable", 500);
        };
        let answers = approvals::answers(&change, body);
        if let Err(why) = crate::core::change::asks::check(&change.asks, &answers) {
            return Response::error(why, 400);
        }
        answers
    } else {
        std::collections::BTreeMap::new()
    };
    let owner = secret(env, "APPROVER")?;
    let hold = body["hold"].as_bool().unwrap_or(false);
    let redeem =
        json!({ "token": token, "operation": entry.operation, "by": owner, "review": hold });
    let (status, v) =
        to_change(env, &s.repo, number, Method::Post, "/redeem", Some(&redeem)).await?;
    let _ = elicit::inbox(
        env,
        s,
        "/approvals/remove",
        &json!({ "path": format!("/approve/{number}?grant={token}") }),
    )
    .await;
    if status != 200 {
        return Response::error("the approval is no longer valid", 409);
    }
    let Ok(grant) = serde_json::from_value::<Grant>(v) else {
        return Response::error("the grant was unreadable", 500);
    };
    let mut args = Map::new();
    args.insert("number".into(), Value::from(number));
    if !answers.is_empty() {
        args.insert("answers".into(), json!(answers));
    }
    if grant.operation == "change.approve"
        && let Some(head) = &grant.head
    {
        args.insert("sha".into(), Value::from(head.clone()));
        if hold {
            args.insert("hold".into(), Value::from(true));
        }
    }
    let request = wire::Request {
        operation: grant.operation.clone(),
        args,
    };
    let who = Identity {
        subject: owner,
        human: true,
    };
    match run(env, s, &who, &request).await {
        Ok(r) if r.ok => Response::from_json(&json!({
            "done": grant.operation, "number": number, "result": r.result,
        })),
        Ok(r) => Response::error(r.violations.join("\n"), 409),
        Err(e) => Response::error(e.to_string(), 409),
    }
}

/// The Auto window running or paused, for the app's home.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_state(env: &Env, s: &Settings) -> Result<Value> {
    let (_, v) = super::mandates::to_instance(env, s, "/windows/current", &json!({})).await?;
    Ok(v["window"].clone())
}

/// `POST /v0/app/auto` with `{hours, passkey}`: starts an Auto window, proved by the owner's
/// passkey for `window:start`.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_start(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let form = approvals::proof_form(body);
    if let Err(why) =
        super::passkeys::prove(env, s, &form, windows::START, &relying_party(env)).await?
    {
        return Response::error(format!("refused: {why}"), 403);
    }
    let owner = secret(env, "APPROVER")?;
    let hours = body["hours"].as_u64().unwrap_or_default();
    let (status, v) = super::mandates::to_instance(
        env,
        s,
        "/windows/start",
        &json!({ "hours": hours, "by": owner }),
    )
    .await?;
    if status != 200 {
        return Response::error(v.as_str().unwrap_or("Auto didn't start").to_string(), 409);
    }
    let said = format!(
        "Auto window {} started for {hours} hours",
        v["window"]["id"].as_u64().unwrap_or_default()
    );
    let _ = super::events::append(env, s, "auto.started", None, None, &said, false).await;
    Response::from_json(&v)
}

/// `DELETE /v0/app/auto` with `{passkey}`: ends the Auto window, proved by the owner's passkey
/// for `window:end`.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_end(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let form = approvals::proof_form(body);
    if let Err(why) =
        super::passkeys::prove(env, s, &form, windows::END, &relying_party(env)).await?
    {
        return Response::error(format!("refused: {why}"), 403);
    }
    let (status, v) = super::mandates::to_instance(
        env,
        s,
        "/windows/end",
        &json!({ "why": "the owner ended it" }),
    )
    .await?;
    if status != 200 {
        return Response::error(
            v.as_str()
                .unwrap_or("no Auto window is running")
                .to_string(),
            409,
        );
    }
    let said = format!(
        "Auto window {} ended by the owner",
        v["window"]["id"].as_u64().unwrap_or_default()
    );
    let _ = super::events::append(env, s, "auto.ended", None, None, &said, false).await;
    Response::from_json(&v)
}
