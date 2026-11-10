//! The GitHub App's installation tokens, and the refs a remote advertises; see
//! `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md` §1, §3.

use serde_json::Value;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit};

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
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        Ok(v)
    } else {
        Err(format!(
            "GitHub {path}: {status} {}",
            v["message"].as_str().unwrap_or_default()
        ))
    }
}

/// An installation token for `owner/repo` only, with what a mirror push needs; minted per job.
///
/// # Errors
///
/// When the App's secrets are missing or GitHub refuses; never with a token.
pub async fn installation_token(env: &Env, owner: &str, repo: &str) -> Result<String, String> {
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
        Some(&mirroring::token_request(repo)),
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
