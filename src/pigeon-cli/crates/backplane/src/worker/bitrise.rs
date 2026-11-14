//! The Bitrise API, for macOS jobs moved off GitHub: trigger, read and abort builds, download
//! the archive, and read the app's `bitrise.yml`; see
//! `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §3 and §4.

use serde_json::{Value, json};
use worker::wasm_bindgen::JsValue;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit};

use super::platform::secret;

const API: &str = "https://api.bitrise.io/v0.1";
/// A Workspace API token of a workspace holding only the pigeon app.
const TOKEN: &str = "BITRISE_TOKEN";
/// The app's slug, kept out of the tree.
const APP: &str = "BITRISE_APP_SLUG";
/// The workflows of `tooling/ci/bitrise/bitrise.yml`.
pub const CHECK_WORKFLOW: &str = "ios-check";
/// The archive's workflow.
pub const ARCHIVE_WORKFLOW: &str = "ios-archive";
/// The archive, as `ios-archive` deploys it.
pub const ARTIFACT: &str = "Pigeon.xcarchive.zip";
/// `main`'s `bitrise.yml`, which the app must hold.
pub const CONFIG: &str = include_str!("../../../../../../tooling/ci/bitrise/bitrise.yml");

fn creds(env: &Env) -> Result<(String, String), String> {
    let token = secret(env, TOKEN).map_err(|_| format!("the secret `{TOKEN}` is not set"))?;
    let app = secret(env, APP).map_err(|_| format!("the secret `{APP}` is not set"))?;
    Ok((token, app))
}

/// `path` under the app, answered as text; the slug never appears in an error.
async fn call(
    env: &Env,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<String, String> {
    let (token, app) = creds(env)?;
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("authorization", &token)?;
    set("accept", "application/json")?;
    let mut init = RequestInit::new();
    if let Some(b) = body {
        set("content-type", "application/json")?;
        init.with_body(Some(JsValue::from_str(&b.to_string())));
    }
    init.with_method(method).with_headers(headers);
    let req = Request::new_with_init(&format!("{API}/apps/{app}{path}"), &init)
        .map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| format!("Bitrise: no answer: {e}"))?;
    let status = r.status_code();
    let text = r.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        Ok(text)
    } else {
        let why = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| "no message".into());
        Err(format!("Bitrise answered {status}: {why}"))
    }
}

async fn call_json(
    env: &Env,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let text = call(env, method, path, body).await?;
    serde_json::from_str(&text).map_err(|e| format!("Bitrise's answer isn't JSON: {e}"))
}

/// The body that triggers `workflow` at `sha` of `branch`, with `envs` as the build's
/// environment.
#[must_use]
pub fn trigger_body(workflow: &str, sha: &str, branch: &str, envs: &[(&str, String)]) -> Value {
    let environments: Vec<Value> = envs
        .iter()
        .map(|(k, v)| json!({ "mapped_to": k, "value": v, "is_expand": false }))
        .collect();
    json!({
        "hook_info": { "type": "bitrise" },
        "build_params": {
            "workflow_id": workflow,
            "commit_hash": sha,
            "branch": branch,
            "environments": environments,
        },
    })
}

/// Triggers `workflow` at `sha`: the build's slug and page.
///
/// # Errors
///
/// When Bitrise refuses, or answers no slug.
pub async fn trigger(
    env: &Env,
    workflow: &str,
    sha: &str,
    branch: &str,
    envs: &[(&str, String)],
) -> Result<(String, Option<String>), String> {
    let body = trigger_body(workflow, sha, branch, envs);
    let answer = call_json(env, Method::Post, "/builds", Some(&body)).await?;
    let slug = answer["build_slug"]
        .as_str()
        .ok_or("Bitrise's answer has no build slug")?;
    Ok((
        slug.to_string(),
        answer["build_url"].as_str().map(str::to_string),
    ))
}

/// Build `slug`: its `data`, which [`crate::macos::bitrise`] reads.
///
/// # Errors
///
/// When Bitrise can't be read.
pub async fn build(env: &Env, slug: &str) -> Result<Value, String> {
    let answer = call_json(env, Method::Get, &format!("/builds/{slug}"), None).await?;
    Ok(answer["data"].clone())
}

/// Aborts build `slug`, for `why`, without notifying anyone.
///
/// # Errors
///
/// When Bitrise refuses.
pub async fn abort(env: &Env, slug: &str, why: &str) -> Result<(), String> {
    let body =
        json!({ "abort_reason": why, "abort_with_success": false, "skip_notifications": true });
    call(
        env,
        Method::Post,
        &format!("/builds/{slug}/abort"),
        Some(&body),
    )
    .await
    .map(drop)
}

/// Build `slug`'s artifact [`ARTIFACT`]: listed, then downloaded from its expiring URL.
///
/// # Errors
///
/// When the build has no such artifact, or a request fails.
pub async fn artifact(env: &Env, slug: &str) -> Result<Vec<u8>, String> {
    let listed = call_json(env, Method::Get, &format!("/builds/{slug}/artifacts"), None).await?;
    let id = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["title"] == ARTIFACT)
        .and_then(|a| a["slug"].as_str())
        .ok_or_else(|| format!("the build has no artifact `{ARTIFACT}`"))?;
    let shown = call_json(
        env,
        Method::Get,
        &format!("/builds/{slug}/artifacts/{id}"),
        None,
    )
    .await?;
    let url = shown["data"]["expiring_download_url"]
        .as_str()
        .ok_or("the artifact has no download URL")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Get);
    let req = Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&r.status_code()) {
        return Err(format!("the artifact's download: {}", r.status_code()));
    }
    r.bytes().await.map_err(|e| e.to_string())
}

/// Why the app's `bitrise.yml` can't be used, if it can't: it must be stored on bitrise.io and
/// equal [`CONFIG`].
///
/// # Errors
///
/// When Bitrise can't be read.
pub async fn config_mismatch(env: &Env) -> Result<Option<String>, String> {
    let held = call(env, Method::Get, "/bitrise.yml", None).await?;
    if let Ok(v) = serde_json::from_str::<Value>(&held)
        && (v["uses_repository_yml"] == true || v["data"]["uses_repository_yml"] == true)
    {
        return Ok(Some("its bitrise.yml is read from the repository".into()));
    }
    Ok((!crate::macos::same_config(&held, CONFIG))
        .then(|| "its bitrise.yml differs from main's tooling/ci/bitrise/bitrise.yml".into()))
}
