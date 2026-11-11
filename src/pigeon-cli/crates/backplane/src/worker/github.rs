//! The GitHub App's installation tokens, and the refs a remote advertises; see
//! `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md` §1, §3.

use serde_json::Value;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, RequestRedirect};

use super::platform::{now, rs256_sign, secret};
use crate::mirroring;

const API: &str = "https://api.github.com";

/// The Worker var naming the GitHub mirror; unset, nothing is mirrored.
pub const MIRROR_VAR: &str = "MIRROR";

/// The secrets the App's tokens need.
pub const APP_ID: &str = "GITHUB_APP_ID";
/// The App's private key, PKCS#8 PEM.
pub const APP_KEY: &str = "GITHUB_APP_KEY";

/// The GitHub mirror's remote, if this Worker mirrors.
#[must_use]
pub fn mirror(env: &Env) -> Option<String> {
    env.var(MIRROR_VAR)
        .ok()
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty())
}

async fn call(
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("authorization", &format!("Bearer {bearer}"))?;
    set("accept", "application/vnd.github+json")?;
    set("x-github-api-version", "2022-11-28")?;
    set("user-agent", "pigeon-control")?;
    let mut init = RequestInit::new();
    if let Some(b) = body {
        set("content-type", "application/json")?;
        init.with_body(Some(b.to_string().into()));
    }
    init.with_method(method).with_headers(headers);
    let req = Request::new_with_init(&format!("{API}{path}"), &init).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    let v: Value = if status == 204 {
        Value::Null
    } else {
        r.json().await.unwrap_or(Value::Null)
    };
    if (200..300).contains(&status) {
        Ok(v)
    } else {
        Err(format!(
            "GitHub {path}: {status} {}",
            v["message"].as_str().unwrap_or_default()
        ))
    }
}

/// An installation token for `owner/repo` only, with the permissions `request` asks for, such
/// as [`mirroring::token_request`]'s; minted per job.
///
/// # Errors
///
/// When the App's secrets are missing or GitHub refuses; never with a token.
pub async fn installation_token(
    env: &Env,
    owner: &str,
    repo: &str,
    request: &Value,
) -> Result<String, String> {
    let id = secret(env, APP_ID).map_err(|_| format!("the secret `{APP_ID}` is not set"))?;
    let pem = secret(env, APP_KEY).map_err(|_| format!("the secret `{APP_KEY}` is not set"))?;
    let der = mirroring::pkcs8_der(&pem)?;
    let input = mirroring::jwt_input(id.trim(), now());
    let signature = rs256_sign(&der, input.as_bytes())
        .await
        .map_err(|e| format!("signing the App's JWT: {e}"))?;
    let jwt = format!("{input}.{}", mirroring::base64url(&signature));
    let installation = call(
        Method::Get,
        &format!("/repos/{owner}/{repo}/installation"),
        &jwt,
        None,
    )
    .await?;
    let n = installation["id"]
        .as_u64()
        .ok_or("the App is not installed on the mirror")?;
    let minted = call(
        Method::Post,
        &format!("/app/installations/{n}/access_tokens"),
        &jwt,
        Some(request),
    )
    .await?;
    minted["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "GitHub's answer holds no token".to_string())
}

/// The refs `remote` advertises over smart HTTP, with `bearer` when given.
///
/// # Errors
///
/// When the request fails or the answer isn't an advertisement.
pub async fn refs(remote: &str, bearer: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("user-agent", "git/2.0 pigeon-control")?;
    if let Some(t) = bearer {
        set("authorization", &format!("Bearer {t}"))?;
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);
    let url = format!(
        "{}/info/refs?service=git-upload-pack",
        remote.trim_end_matches('/')
    );
    let req = Request::new_with_init(&url, &init).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    if !(200..300).contains(&status) {
        return Err(format!("info/refs answered {status}"));
    }
    let body = r.bytes().await.map_err(|e| e.to_string())?;
    mirroring::advertised(&body).ok_or_else(|| "info/refs is not an advertisement".to_string())
}

/// Dispatches `main`'s `ci.yml` on `owner/repo` with `body`, from
/// [`crate::actions::dispatch_body`].
///
/// # Errors
///
/// When GitHub refuses, with its status and message.
pub async fn dispatch(token: &str, owner: &str, repo: &str, body: &Value) -> Result<(), String> {
    let path = format!(
        "/repos/{owner}/{repo}/actions/workflows/{}/dispatches",
        crate::actions::WORKFLOW
    );
    call(Method::Post, &path, token, Some(body))
        .await
        .map(|_| ())
}

/// The newest dispatched runs of `ci.yml` on `main`, as GitHub lists them.
///
/// # Errors
///
/// When GitHub refuses.
pub async fn runs(token: &str, owner: &str, repo: &str) -> Result<Value, String> {
    let path = format!(
        "/repos/{owner}/{repo}/actions/workflows/{}/runs?event=workflow_dispatch&branch={}&per_page=50",
        crate::actions::WORKFLOW,
        crate::actions::REF
    );
    call(Method::Get, &path, token, None).await
}

/// Run `id` and its jobs.
///
/// # Errors
///
/// When GitHub refuses.
pub async fn run(token: &str, owner: &str, repo: &str, id: u64) -> Result<(Value, Value), String> {
    let base = format!("/repos/{owner}/{repo}/actions/runs/{id}");
    let run = call(Method::Get, &base, token, None).await?;
    let jobs = call(
        Method::Get,
        &format!("{base}/jobs?per_page=100"),
        token,
        None,
    )
    .await?;
    Ok((run, jobs))
}

/// Job `id`'s log: GitHub answers with a redirect to it, followed without the token.
///
/// # Errors
///
/// When either request fails.
pub async fn job_log(token: &str, owner: &str, repo: &str, id: u64) -> Result<String, String> {
    let path = format!("/repos/{owner}/{repo}/actions/jobs/{id}/logs");
    let mut r = redirected(token, &path, &format!("job {id}'s log")).await?;
    r.text().await.map_err(|e| e.to_string())
}

/// Run `run`'s artifact `name`, uploaded unzipped: listed with the token, then downloaded from
/// GitHub's redirect without it.
///
/// # Errors
///
/// When the run has no such artifact, or a request fails.
pub async fn artifact(
    token: &str,
    owner: &str,
    repo: &str,
    run: u64,
    name: &str,
) -> Result<Vec<u8>, String> {
    let base = format!("/repos/{owner}/{repo}/actions");
    let listed = call(
        Method::Get,
        &format!("{base}/runs/{run}/artifacts?name={name}"),
        token,
        None,
    )
    .await?;
    let id = listed["artifacts"][0]["id"]
        .as_u64()
        .ok_or_else(|| format!("run {run} has no artifact `{name}`"))?;
    let what = format!("artifact `{name}`");
    let mut r = redirected(token, &format!("{base}/artifacts/{id}/zip"), &what).await?;
    r.bytes().await.map_err(|e| e.to_string())
}

/// `path`'s answer, a redirect, followed without the token; `what` names it in errors.
async fn redirected(token: &str, path: &str, what: &str) -> Result<worker::Response, String> {
    let headers = Headers::new();
    let set = |k: &str, v: &str| headers.set(k, v).map_err(|e| e.to_string());
    set("authorization", &format!("Bearer {token}"))?;
    set("accept", "application/vnd.github+json")?;
    set("x-github-api-version", "2022-11-28")?;
    set("user-agent", "pigeon-control")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Get)
        .with_headers(headers)
        .with_redirect(RequestRedirect::Manual);
    let req = Request::new_with_init(&format!("{API}{path}"), &init).map_err(|e| e.to_string())?;
    let r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let location = r
        .headers()
        .get("location")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{what}: {} and no redirect", r.status_code()))?;
    let mut plain = RequestInit::new();
    plain.with_method(Method::Get);
    let req = Request::new_with_init(&location, &plain).map_err(|e| e.to_string())?;
    let r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&r.status_code()) {
        return Err(format!("{what}: {}", r.status_code()));
    }
    Ok(r)
}
