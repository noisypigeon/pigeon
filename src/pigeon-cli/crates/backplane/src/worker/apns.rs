//! APNs from the control plane: a provider token signed here and reused, and an alert to each
//! linked device; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §6.

use serde_json::{Value, json};
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, Result};

use super::platform::{Settings, now, secret};
use crate::core::app::push;
use crate::devices::Device;

/// The `.p8` key, PEM.
const KEY: &str = "APNS_KEY";
/// Its key ID.
const KEY_ID: &str = "APNS_KEY_ID";

/// A provider token signed now, or none without the key.
async fn provider_token(env: &Env) -> Result<Option<String>> {
    let (Ok(pem), Ok(kid), Ok(team)) = (
        secret(env, KEY),
        secret(env, KEY_ID),
        secret(env, "APPLE_TEAM_ID"),
    ) else {
        return Ok(None);
    };
    let der = crate::apple::pem_der(&pem).map_err(worker::Error::from)?;
    let input = push::token_input(kid.trim(), team.trim(), now());
    let signature = super::apple::es256_sign(&der, input.as_bytes()).await?;
    Ok(Some(crate::apple::jwt(&input, &signature)))
}

/// The provider token, reused for [`push::TOKEN_TTL`] in the `Instance`.
async fn token(env: &Env, s: &Settings) -> Result<Option<String>> {
    let (_, cached) = super::mandates::to_instance(env, s, "/apns/token", &json!({})).await?;
    if let (Some(t), Some(at)) = (cached["token"].as_str(), cached["at"].as_u64())
        && now().saturating_sub(at) < push::TOKEN_TTL
    {
        return Ok(Some(t.to_string()));
    }
    let Some(t) = provider_token(env).await? else {
        return Ok(None);
    };
    super::mandates::to_instance(env, s, "/apns/token", &json!({ "token": t, "at": now() }))
        .await?;
    Ok(Some(t))
}

/// Sends `title` and `body`, linked to `path`, to every linked device with a token; drops a
/// token APNs says is gone. Nothing happens without the key.
///
/// # Errors
///
/// When a binding fails.
pub async fn notify(env: &Env, s: &Settings, title: &str, body: &str, path: &str) -> Result<()> {
    let v = super::elicit::inbox(env, s, "/devices/list", &json!({})).await?;
    let devices: Vec<Device> = serde_json::from_value(v["devices"].clone()).unwrap_or_default();
    let targets: Vec<(String, String)> = devices
        .into_iter()
        .filter_map(|d| d.push.map(|t| (d.id, t)))
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    let Some(bearer) = token(env, s).await? else {
        return Ok(());
    };
    let payload = push::payload(title, body, path).to_string();
    for (id, device_token) in targets {
        let headers = Headers::new();
        headers.set("authorization", &format!("bearer {bearer}"))?;
        headers.set("apns-topic", push::TOPIC)?;
        headers.set("apns-push-type", "alert")?;
        headers.set("apns-priority", "10")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(payload.clone().into()));
        let url = format!("{}/3/device/{device_token}", push::HOST);
        let req = Request::new_with_init(&url, &init)?;
        let mut r = Fetch::Request(req).send().await?;
        let status = r.status_code();
        if status != 200 {
            let answer: Value = r.json().await.unwrap_or(Value::Null);
            let reason = answer["reason"].as_str().unwrap_or_default();
            worker::console_warn!("apns: {status} {reason} for device {id}");
            if push::gone(status, reason) {
                super::elicit::inbox(env, s, "/devices/push", &json!({ "id": id, "token": "" }))
                    .await?;
            }
        }
    }
    Ok(())
}

/// `/apns/token` on the `Instance`: the cached provider token, or caches the one given.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &worker::State, body: Value) -> Result<worker::Response> {
    let storage = state.storage();
    if body["token"].is_string() {
        storage.put("apns_token", &body).await?;
        return worker::Response::from_json(&json!({ "status": "ok" }));
    }
    let cached: Value = storage.get("apns_token").await?.unwrap_or(Value::Null);
    worker::Response::from_json(&cached)
}
