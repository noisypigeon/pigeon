//! The Apple connection: App Store Connect tokens signed here, the `sign` step's grant at
//! `POST /v0/apple/release`, and `apple.check`; see
//! `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md` §5.

use serde_json::{Value, json};
use worker::js_sys::{self, Array, Function, Promise, Reflect, Uint8Array};
use worker::wasm_bindgen::{JsCast, JsValue};
use worker::wasm_bindgen_futures::JsFuture;
use worker::{Env, Fetch, Method, Request, Response, Result};

use super::platform::{d1, d1_rows, json_request, now, rs256, secret};
use crate::access;
use crate::apple::{self, Job};
use crate::signing::{Grant, PLATFORM};

/// The App Store Connect key, `.p8`.
const ASC_KEY: &str = "APPLE_ASC_KEY";
/// Its key ID.
const ASC_KEY_ID: &str = "APPLE_ASC_KEY_ID";
/// The team's issuer ID.
const ASC_ISSUER: &str = "APPLE_ASC_ISSUER";
/// The developer team.
const TEAM_ID: &str = "APPLE_TEAM_ID";
/// The distribution certificate and key, `.p12`, base64.
const DIST_P12: &str = "APPLE_DIST_P12";
/// Its password.
const DIST_P12_PASSWORD: &str = "APPLE_DIST_P12_PASSWORD";

fn subtle() -> Result<JsValue> {
    let crypto = Reflect::get(&js_sys::global(), &"crypto".into())?;
    Ok(Reflect::get(&crypto, &"subtle".into())?)
}

fn method(on: &JsValue, name: &str) -> Result<Function> {
    Ok(Reflect::get(on, &name.into())?.dyn_into()?)
}

async fn promise(v: JsValue) -> Result<JsValue> {
    Ok(JsFuture::from(v.dyn_into::<Promise>()?).await?)
}

/// `data` signed ES256 with the PKCS#8 P-256 key `der`: the raw `r‖s` JWS takes.
async fn es256_sign(der: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let subtle = subtle()?;
    let import_alg = js_sys::Object::new();
    Reflect::set(&import_alg, &"name".into(), &"ECDSA".into())?;
    Reflect::set(&import_alg, &"namedCurve".into(), &"P-256".into())?;
    let usages = Array::of1(&"sign".into());
    let import = method(&subtle, "importKey")?;
    let args = Array::of5(
        &"pkcs8".into(),
        &Uint8Array::from(der),
        &import_alg,
        &JsValue::FALSE,
        &usages,
    );
    let key = promise(import.apply(&subtle, &args)?).await?;
    let sign_alg = js_sys::Object::new();
    Reflect::set(&sign_alg, &"name".into(), &"ECDSA".into())?;
    Reflect::set(&sign_alg, &"hash".into(), &"SHA-256".into())?;
    let sign = method(&subtle, "sign")?;
    let args = Array::of3(&sign_alg, &key, &Uint8Array::from(data));
    let signed = promise(sign.apply(&subtle, &args)?).await?;
    Ok(Uint8Array::new(&signed).to_vec())
}

/// An App Store Connect token valid for `ttl` seconds, at most 20 minutes.
async fn asc_token(env: &Env, ttl: u64) -> Result<String> {
    let der = apple::pem_der(&secret(env, ASC_KEY)?).map_err(worker::Error::from)?;
    let input = apple::asc_jwt_input(
        &secret(env, ASC_KEY_ID)?,
        &secret(env, ASC_ISSUER)?,
        now(),
        ttl,
    );
    let signature = es256_sign(&der, input.as_bytes()).await?;
    Ok(apple::jwt(&input, &signature))
}

/// `GET` an App Store Connect path with a fresh token.
pub(super) async fn asc_get(env: &Env, path: &str) -> std::result::Result<Value, String> {
    let token = asc_token(env, 300).await.map_err(|e| e.to_string())?;
    let url = format!("{}{path}", apple::ASC_API);
    let req = json_request(&url, Method::Get, None, Some(&token)).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let detail = body["errors"][0]["detail"].as_str().unwrap_or("no detail");
        return Err(format!(
            "App Store Connect answered {status} for {path}: {detail}"
        ));
    }
    Ok(body)
}

/// The app's active App Store profile: `(uuid, name, content)`.
async fn profile(env: &Env) -> std::result::Result<(String, String, String), String> {
    let ids = asc_get(
        env,
        &format!("/v1/bundleIds?filter[identifier]={}", apple::BUNDLE_ID),
    )
    .await?;
    let id = apple::bundle_id(&ids)?;
    let profiles = asc_get(env, &format!("/v1/bundleIds/{id}/profiles?limit=200")).await?;
    apple::app_store_profile(&profiles)
}

/// The verified Buildkite job behind `req`'s bearer token, or why not.
async fn job(req: &Request) -> std::result::Result<Job, String> {
    let header = req
        .headers()
        .get("authorization")
        .ok()
        .flatten()
        .ok_or("no OIDC token")?;
    let jwt = header.strip_prefix("Bearer ").ok_or("no OIDC token")?;
    let token = access::decode(jwt)?;
    if token.header["alg"] != "RS256" {
        return Err("the OIDC token must be RS256".into());
    }
    let url = worker::Url::parse(apple::BUILDKITE_JWKS).map_err(|e| e.to_string())?;
    let mut r = Fetch::Url(url).send().await.map_err(|e| e.to_string())?;
    let keys: Value = r.json().await.map_err(|e| e.to_string())?;
    let key = access::key(&token, &keys)?;
    if !rs256(key, token.signing_input.as_bytes(), &token.signature)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("the OIDC token's signature does not verify".into());
    }
    apple::check_claims(&token.claims, PLATFORM, now())
}

/// `POST /v0/apple/release`: a `sign` step's OIDC token for its release's grant, once.
/// Access lets this path through, so this alone authenticates the caller.
///
/// # Errors
///
/// When D1 fails; every refusal is a response.
pub async fn release(req: Request, env: &Env) -> Result<Response> {
    let job = match job(&req).await {
        Ok(job) => job,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    let redeemed = d1_rows(env, &apple::redeem(&job, now())).await?;
    let Some(build) = redeemed.first().and_then(|r| r["build"].as_u64()) else {
        return Response::error(
            "no archived, unredeemed release for this build and commit",
            403,
        );
    };
    match grant(env).await {
        Ok(grant) => Response::from_json(&grant),
        Err(why) => {
            d1(env, &[apple::fail(build, &why, now())]).await?;
            Response::error(format!("the grant failed: {why}"), 502)
        }
    }
}

async fn grant(env: &Env) -> std::result::Result<Grant, String> {
    let s = |name: &str| secret(env, name).map_err(|e| e.to_string());
    let (profile_uuid, profile_name, content) = profile(env).await?;
    let upload_jwt = asc_token(env, apple::ASC_MAX_TTL)
        .await
        .map_err(|e| e.to_string())?;
    let grant = Grant {
        p12: apple::to_b64url(&s(DIST_P12)?)?,
        p12_password: s(DIST_P12_PASSWORD)?,
        profile: apple::to_b64url(&content)?,
        profile_uuid: profile_uuid.to_ascii_uppercase(),
        profile_name,
        team_id: s(TEAM_ID)?,
        bundle_id: apple::BUNDLE_ID.into(),
        upload_jwt,
        expires: now() + apple::ASC_MAX_TTL,
    };
    grant.validate()?;
    Ok(grant)
}

/// `apple.check`: whether the key signs tokens App Store Connect accepts, the app and its
/// profile exist, and when the profile expires. It never shows a secret.
///
/// # Errors
///
/// Never; each failure is a line of the answer.
pub async fn check(env: &Env) -> Result<Value> {
    let mut checks = serde_json::Map::new();
    let mut ok = true;
    let mut record = |name: &str, r: std::result::Result<Value, String>| {
        ok &= r.is_ok();
        checks.insert(
            name.into(),
            match r {
                Ok(v) => json!({ "ok": true, "detail": v }),
                Err(e) => json!({ "ok": false, "detail": e }),
            },
        );
    };
    for name in [
        ASC_KEY,
        ASC_KEY_ID,
        ASC_ISSUER,
        TEAM_ID,
        DIST_P12,
        DIST_P12_PASSWORD,
    ] {
        record(
            &format!("secret {name}"),
            secret(env, name)
                .map(|_| json!("set"))
                .map_err(|e| e.to_string()),
        );
    }
    let app = asc_get(
        env,
        &format!("/v1/apps?filter[bundleId]={}", apple::BUNDLE_ID),
    )
    .await;
    record(
        "app",
        app.and_then(|a| {
            a["data"][0]["attributes"]["name"]
                .as_str()
                .map(|n| json!(n))
                .ok_or_else(|| format!("no app for {}", apple::BUNDLE_ID))
        }),
    );
    let ids = asc_get(
        env,
        &format!("/v1/bundleIds?filter[identifier]={}", apple::BUNDLE_ID),
    )
    .await;
    let profiles = match ids.and_then(|i| apple::bundle_id(&i)) {
        Ok(id) => asc_get(env, &format!("/v1/bundleIds/{id}/profiles?limit=200")).await,
        Err(e) => Err(e),
    };
    record(
        "profile",
        profiles.and_then(|p| {
            let (_, name, _) = apple::app_store_profile(&p)?;
            let expires = p["data"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|d| d["attributes"]["name"] == name.as_str())
                .and_then(|d| d["attributes"]["expirationDate"].as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(json!({ "name": name, "expires": expires }))
        }),
    );
    Ok(json!({ "ok": ok, "checks": checks }))
}

/// `GET /v0/apple/release/archive`: the release's archive, to the same `sign` step that redeemed
/// its grant, while it signs; see the 2026-11-12 amendment.
///
/// # Errors
///
/// When D1 or R2 fails; every refusal is a response.
pub async fn archive(req: Request, env: &Env) -> Result<Response> {
    let job = match job(&req).await {
        Ok(job) => job,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    let rows = d1_rows(env, &crate::iosrelease::signing_row(job.build, &job.commit)).await?;
    let Some(build) = rows.first().and_then(|r| r["build"].as_u64()) else {
        return Response::error("no release of this build and commit is signing", 403);
    };
    let key = crate::iosrelease::key(&job.commit, build);
    let Some(object) = env.bucket("CI_LOGS")?.get(key).execute().await? else {
        return Response::error("the archive is missing", 404);
    };
    let Some(body) = object.body() else {
        return Response::error("the archive is missing", 404);
    };
    let mut r = Response::from_stream(body.stream()?)?;
    r.headers_mut().set("content-type", "application/zip")?;
    Ok(r)
}

/// App Store Connect's processing state of release `build`; none until it lists the build.
///
/// # Errors
///
/// When App Store Connect can't be read.
pub(super) async fn processing(
    env: &Env,
    build: u64,
) -> std::result::Result<Option<String>, String> {
    let apps = asc_get(
        env,
        &format!("/v1/apps?filter[bundleId]={}", apple::BUNDLE_ID),
    )
    .await?;
    let app = apps["data"][0]["id"]
        .as_str()
        .ok_or_else(|| format!("no app for {}", apple::BUNDLE_ID))?;
    let builds = asc_get(
        env,
        &format!("/v1/builds?filter[app]={app}&filter[version]={build}&limit=1"),
    )
    .await?;
    Ok(crate::iosrelease::processing_state(&builds).map(String::from))
}
