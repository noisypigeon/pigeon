//! The Buildkite REST API, for a release's signing build; see the 2026-11-12 amendment to
//! `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`.

use serde_json::Value;
use worker::{Env, Fetch, Method};

use super::platform::{json_request, secret};
use crate::iosrelease;

const API: &str = "https://api.buildkite.com";
/// The API token, scoped to the signing pipeline's organization.
const TOKEN: &str = "BUILDKITE_TOKEN";

async fn call(
    env: &Env,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let token = secret(env, TOKEN).map_err(|e| e.to_string())?;
    let req = json_request(&format!("{API}{path}"), method, body, Some(&token))
        .map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    let answer: Value = r.json().await.unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        Ok(answer)
    } else {
        let why = answer["message"].as_str().unwrap_or("no message");
        Err(format!("Buildkite answered {status}: {why}"))
    }
}

/// Creates release `build`'s signing build of `sha`: its number.
///
/// # Errors
///
/// When Buildkite refuses, or answers no number.
pub async fn create(env: &Env, build: u64, sha: &str) -> Result<u64, String> {
    let body = iosrelease::buildkite_body(build, sha);
    let answer = call(
        env,
        Method::Post,
        &iosrelease::buildkite_path(None),
        Some(&body),
    )
    .await?;
    answer["number"]
        .as_u64()
        .ok_or_else(|| "Buildkite's answer has no build number".into())
}

/// The state of signing build `number`: `scheduled`, `running`, `passed`, `failed` and so on.
///
/// # Errors
///
/// When Buildkite can't be read.
pub async fn state(env: &Env, number: u64) -> Result<String, String> {
    let answer = call(
        env,
        Method::Get,
        &iosrelease::buildkite_path(Some(number)),
        None,
    )
    .await?;
    answer["state"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Buildkite's answer has no state".into())
}

/// The pipeline that runs a moved macOS job; see
/// `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §3.
pub const BUILD_PIPELINE: &str = "pigeon-ios-build";
/// `main`'s steps for it, which its settings must hold.
pub const BUILD_CONFIG: &str = include_str!("../../../../../../tooling/ci/buildkite/ios-build.yml");

fn build_path(rest: &str) -> String {
    format!(
        "/v2/organizations/{}/pipelines/{BUILD_PIPELINE}{rest}",
        iosrelease::BUILDKITE_ORG
    )
}

/// Creates a `pigeon-ios-build` build of `sha` on `branch` running `ci.sh <job>`, with the
/// release's `build` number for an archive: its number and page.
///
/// # Errors
///
/// When Buildkite refuses, or answers no number.
pub async fn create_build(
    env: &Env,
    job: &str,
    sha: &str,
    branch: &str,
    build: Option<u64>,
) -> Result<(u64, Option<String>), String> {
    let mut vars = serde_json::Map::new();
    vars.insert("PIGEON_JOB".into(), Value::from(job));
    if let Some(b) = build {
        vars.insert("PIGEON_IOS_BUILD".into(), Value::from(b.to_string()));
    }
    let body = serde_json::json!({
        "commit": sha,
        "branch": branch,
        "message": format!("pigeon macOS {job} of {}", &sha[..sha.len().min(12)]),
        "env": vars,
    });
    let answer = call(env, Method::Post, &build_path("/builds"), Some(&body)).await?;
    let number = answer["number"]
        .as_u64()
        .ok_or("Buildkite's answer has no build number")?;
    Ok((number, answer["web_url"].as_str().map(String::from)))
}

/// `pigeon-ios-build` build `number`'s one job, which [`crate::macos::buildkite`] reads.
///
/// # Errors
///
/// When Buildkite can't be read.
pub async fn build_job(env: &Env, number: u64) -> Result<Value, String> {
    let answer = call(
        env,
        Method::Get,
        &build_path(&format!("/builds/{number}")),
        None,
    )
    .await?;
    let job = answer["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|j| j["type"] == "script")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "state": answer["state"].clone() }));
    Ok(job)
}

/// Cancels `pigeon-ios-build` build `number`.
///
/// # Errors
///
/// When Buildkite refuses.
pub async fn cancel(env: &Env, number: u64) -> Result<(), String> {
    call(
        env,
        Method::Put,
        &build_path(&format!("/builds/{number}/cancel")),
        None,
    )
    .await
    .map(drop)
}

/// Build `number`'s job `job` artifact `name`: listed with the token, then downloaded from
/// Buildkite's redirect without it.
///
/// # Errors
///
/// When there is no such artifact, or a request fails.
pub async fn artifact(env: &Env, number: u64, job: &str, name: &str) -> Result<Vec<u8>, String> {
    let listed = call(
        env,
        Method::Get,
        &build_path(&format!("/builds/{number}/jobs/{job}/artifacts")),
        None,
    )
    .await?;
    let url = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["filename"] == name)
        .and_then(|a| a["download_url"].as_str())
        .ok_or_else(|| format!("the job has no artifact `{name}`"))?;
    let token = secret(env, TOKEN).map_err(|e| e.to_string())?;
    let headers = worker::Headers::new();
    headers
        .set("authorization", &format!("Bearer {token}"))
        .map_err(|e| e.to_string())?;
    let mut init = worker::RequestInit::new();
    init.with_method(Method::Get)
        .with_headers(headers)
        .with_redirect(worker::RequestRedirect::Manual);
    let req = worker::Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
    let r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let location = r
        .headers()
        .get("location")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("the artifact: {} and no redirect", r.status_code()))?;
    let mut plain = worker::RequestInit::new();
    plain.with_method(Method::Get);
    let req = worker::Request::new_with_init(&location, &plain).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&r.status_code()) {
        return Err(format!("the artifact's download: {}", r.status_code()));
    }
    r.bytes().await.map_err(|e| e.to_string())
}

/// Why `pigeon-ios-build` can't be used, if it can't: its steps must be [`BUILD_CONFIG`].
///
/// # Errors
///
/// When Buildkite can't be read.
pub async fn config_mismatch(env: &Env) -> Result<Option<String>, String> {
    let p = call(env, Method::Get, &build_path(""), None).await?;
    let held = p["configuration"].as_str().unwrap_or_default();
    Ok((!crate::macos::same_config(held, BUILD_CONFIG))
        .then(|| "its steps differ from main's tooling/ci/buildkite/ios-build.yml".to_string()))
}
