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
