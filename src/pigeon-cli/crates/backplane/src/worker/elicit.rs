//! Approvals in the card: a form-mode elicitation asking for the owner's authenticator code,
//! held open as Server-Sent Events while the `Instance` Durable Object waits for the answer; see
//! the amendment of 2026-11-07 to
//! `.claude/decisions/2026-11-06-development-control-plane-approvals-images-state.md` §1.

use std::time::Duration;

use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use worker::{Delay, Env, Headers, Method, Response, Result, State};

use super::platform::{self, Settings, json_request, now};
use super::{run, to_change};
use crate::access::Identity;
use crate::grants::{self, Grant};
use crate::mcp;
use crate::totp::{self, Guard};
use crate::wire;

/// The storage key of the one pending approval.
const PENDING: &str = "elicit";

/// The storage key of the code's [`Guard`].
const GUARD: &str = "totp";

/// How long a card waits for its answer, in seconds.
const WAIT: u64 = 600;

/// The one approval waiting for its card's answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    key: String,
    number: u32,
    operation: String,
    token: String,
    #[serde(default)]
    answer: Option<Value>,
}

async fn to_instance(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<Value> {
    let req = json_request(
        &format!("https://instance{path}"),
        Method::Post,
        Some(body),
        None,
    )?;
    let mut r = env
        .durable_object("INSTANCE")?
        .id_from_name(&s.instance)?
        .get_stub()?
        .fetch_with_request(req)
        .await?;
    let text = r.text().await?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn checks(change: &Value) -> String {
    let jobs: Vec<String> = change["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|j| {
            format!(
                "{} {}",
                j["name"].as_str().unwrap_or_default(),
                j["status"].as_str().unwrap_or_default()
            )
        })
        .collect();
    if jobs.is_empty() {
        "none yet".into()
    } else {
        jobs.join(", ")
    }
}

/// Offers `operation` on change `number` as a card asking for the owner's code, and answers the
/// call once the card is answered.
///
/// # Errors
///
/// When the change refuses a grant, or a binding fails.
pub async fn card(
    env: Env,
    id: Value,
    number: u32,
    operation: String,
) -> std::result::Result<Response, String> {
    let s = Settings::from(&env).map_err(|e| e.to_string())?;
    let body = json!({ "operation": operation });
    let (status, v) = to_change(&env, &s.repo, number, Method::Post, "/grant", Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(v.as_str().map_or_else(|| v.to_string(), str::to_string));
    }
    let token = v["token"]
        .as_str()
        .ok_or("the grant has no token")?
        .to_string();
    let grant: Grant = serde_json::from_value(v["grant"].clone()).map_err(|e| e.to_string())?;
    let (_, st) = to_change(&env, &s.repo, number, Method::Get, "/status", None)
        .await
        .map_err(|e| e.to_string())?;
    let change = &st["change"];
    let message = format!(
        "{operation} — change {number}\n{}\nBranch: {}\nHead: {}\nChecks: {}\nProposed by: {}\n\nEnter your authenticator code to confirm.",
        change["title"].as_str().unwrap_or_default(),
        st["branch"].as_str().unwrap_or_default(),
        grant.head.as_deref().unwrap_or("none pushed"),
        checks(change),
        change["proposer"].as_str().unwrap_or_default(),
    );
    let key = format!("{}{number}-{}", mcp::ELICIT, &grants::key(&token)[..12]);
    let pending = Pending {
        key: key.clone(),
        number,
        operation: operation.clone(),
        token,
        answer: None,
    };
    to_instance(&env, &s, "/elicit/open", &json!(pending))
        .await
        .map_err(|e| e.to_string())?;
    let first = mcp::sse(&mcp::elicit_form(&key, &message, &mcp::code_schema()));
    let rest = async move {
        let answer = to_instance(&env, &s, "/elicit/await", &json!({ "key": key }))
            .await
            .unwrap_or_else(|e| json!({ "status": "error", "why": e.to_string() }));
        let outcome = match answer["status"].as_str() {
            Some("ok") => finish(&env, &s, number, &operation, &answer["grant"]).await,
            _ => Err(crate::Error::Invalid(vec![
                answer["why"]
                    .as_str()
                    .unwrap_or("the card was not answered")
                    .to_string(),
            ])),
        };
        Ok::<Vec<u8>, worker::Error>(mcp::sse(&mcp::answer(&id, outcome)).into_bytes())
    };
    let body = stream::once(async move { Ok::<Vec<u8>, worker::Error>(first.into_bytes()) })
        .chain(stream::once(rest));
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("content-type", "text/event-stream")?;
    set("cache-control", "no-cache")?;
    Ok(Response::from_stream(body)
        .map_err(|e| e.to_string())?
        .with_headers(headers))
}

/// Runs a redeemed grant's operation as the owner.
async fn finish(
    env: &Env,
    s: &Settings,
    number: u32,
    operation: &str,
    grant: &Value,
) -> std::result::Result<wire::Response, crate::Error> {
    let invalid = |m: String| crate::Error::Invalid(vec![m]);
    let grant: Grant = serde_json::from_value(grant.clone()).map_err(|e| invalid(e.to_string()))?;
    let approver = platform::secret(env, "APPROVER").map_err(|e| invalid(e.to_string()))?;
    let mut args = Map::new();
    args.insert("number".into(), Value::from(number));
    if operation == "change.approve"
        && let Some(head) = grant.head
    {
        args.insert("sha".into(), Value::from(head));
    }
    let request = wire::Request {
        operation: operation.to_string(),
        args,
    };
    let who = Identity {
        subject: approver,
        human: true,
    };
    run(env, s, &who, &request).await
}

/// Hands the client's answer to card `key` to the `Instance` Durable Object.
///
/// # Errors
///
/// When the binding fails.
pub async fn answer(env: &Env, key: &str, result: &Value) -> Result<Response> {
    let s = Settings::from(env)?;
    let v = to_instance(
        env,
        &s,
        "/elicit/answer",
        &json!({ "key": key, "result": result }),
    )
    .await?;
    worker::console_log!("card {key}: {}", v["status"]);
    Ok(Response::empty()?.with_status(202))
}

/// The `Instance` Durable Object's side: `/elicit/open`, `/elicit/answer` and `/elicit/await`.
///
/// # Errors
///
/// When storage or a binding fails.
pub async fn serve(state: &State, env: &Env, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    match path {
        "/elicit/open" => {
            let pending: Pending = serde_json::from_value(body)?;
            storage.put(PENDING, &pending).await?;
            Response::from_json(&json!({ "status": "open" }))
        }
        "/elicit/answer" => {
            let Some(mut pending) = storage.get::<Pending>(PENDING).await? else {
                return Response::from_json(&json!({ "status": "none waiting" }));
            };
            if body["key"].as_str() != Some(pending.key.as_str()) || pending.answer.is_some() {
                return Response::from_json(&json!({ "status": "not this card" }));
            }
            let answer = judge(state, env, &pending, &body["result"]).await?;
            pending.answer = Some(answer.clone());
            storage.put(PENDING, &pending).await?;
            Response::from_json(&json!({ "status": answer["status"] }))
        }
        "/elicit/await" => {
            let key = body["key"].as_str().unwrap_or_default().to_string();
            for _ in 0..WAIT {
                match storage.get::<Pending>(PENDING).await? {
                    Some(p) if p.key != key => {
                        return Response::from_json(&json!({
                            "status": "superseded",
                            "why": "a newer approval card replaced this one",
                        }));
                    }
                    Some(Pending {
                        answer: Some(a), ..
                    }) => {
                        storage.delete(PENDING).await?;
                        return Response::from_json(&a);
                    }
                    None => {
                        return Response::from_json(&json!({
                            "status": "gone",
                            "why": "this approval card is no longer waiting",
                        }));
                    }
                    Some(_) => Delay::from(Duration::from_secs(1)).await,
                }
            }
            storage.delete(PENDING).await?;
            Response::from_json(&json!({
                "status": "timeout",
                "why": "the card was not answered within 10 minutes",
            }))
        }
        _ => Response::error("not found", 404),
    }
}

/// The outcome of the client's `result` for `pending`: declined, refused with why, or `ok` with
/// the redeemed grant. A card takes one code; a wrong one ends it.
async fn judge(state: &State, env: &Env, pending: &Pending, result: &Value) -> Result<Value> {
    if result["action"].as_str() != Some("accept") {
        return Ok(json!({ "status": "declined", "why": "the owner declined" }));
    }
    let code = result["content"]["code"].as_str().unwrap_or_default();
    let secret = totp::base32_decode(&platform::secret(env, "APPROVER_TOTP")?)?;
    let storage = state.storage();
    let mut guard: Guard = storage.get(GUARD).await?.unwrap_or_default();
    let checked = guard.verify(&secret, code, now());
    storage.put(GUARD, &guard).await?;
    if let Err(why) = checked {
        return Ok(json!({ "status": "refused", "why": why }));
    }
    let s = Settings::from(env)?;
    let body = json!({ "token": pending.token, "operation": pending.operation });
    let (status, v) = to_change(
        env,
        &s.repo,
        pending.number,
        Method::Post,
        "/redeem",
        Some(&body),
    )
    .await?;
    if status == 200 {
        Ok(json!({ "status": "ok", "grant": v }))
    } else {
        Ok(json!({
            "status": "refused",
            "why": v.as_str().map_or_else(|| v.to_string(), str::to_string),
        }))
    }
}
