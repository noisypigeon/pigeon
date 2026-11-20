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
pub(super) async fn es256_sign(der: &[u8], data: &[u8]) -> Result<Vec<u8>> {
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
    let Some(object) = super::builds_bucket(env)?.get(key).execute().await? else {
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

/// The most of the archive one `release.apple.archive` answers with.
const ARCHIVE_PIECE: u64 = 4 * 1024 * 1024;

/// 32 random bytes, for a seal's sender.
fn seed() -> Result<[u8; 32]> {
    let hex = super::platform::random_hex(32)?;
    let mut out = [0; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2).unwrap_or("00"), 16)
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
    }
    Ok(out)
}

/// One JSON-RPC answer to a `tools/call`: `value` as its text and structured content.
fn answer(id: &Value, ok: bool, value: &Value) -> Result<Response> {
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string);
    Response::from_json(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text }],
            "structuredContent": if value.is_object() { value.clone() } else { json!({ "message": value }) },
            "isError": !ok,
        }
    }))
}

/// `POST /v0/app/mcp` with a Buildkite OIDC token: `release.apple` and `release.apple.archive`
/// for the `sign` step, its grant sealed to the step's own key; see
/// `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`, its E2 amendment.
///
/// # Errors
///
/// When D1 or R2 fails; every refusal is an answer.
pub async fn mcp(mut req: Request, env: &Env) -> Result<Response> {
    let job = match job(&req).await {
        Ok(job) => job,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let id = body["id"].clone();
    if body["method"] != "tools/call" {
        return Response::error(
            "this door answers `tools/call` alone for a Buildkite step",
            400,
        );
    }
    let args = &body["params"]["arguments"];
    match body["params"]["name"].as_str().unwrap_or_default() {
        "release_apple" => sealed_release(env, &job, &id, args).await,
        "release_apple_archive" => archive_piece(env, &job, &id, args).await,
        other => answer(
            &id,
            false,
            &json!(format!("no tool `{other}` for a Buildkite step")),
        ),
    }
}

/// `release.apple {recipient}`: redeems the release once and answers its grant sealed to
/// `recipient`, logging the key's fingerprint and never the grant.
async fn sealed_release(env: &Env, job: &Job, id: &Value, args: &Value) -> Result<Response> {
    let Some(recipient) = args["recipient"].as_str().filter(|r| !r.is_empty()) else {
        return answer(
            id,
            false,
            &json!("`recipient` is the step's X25519 public key, base64url"),
        );
    };
    let Ok(public) = access::base64url(recipient) else {
        return answer(id, false, &json!("`recipient` is not base64url"));
    };
    let redeemed = d1_rows(env, &apple::redeem(job, now())).await?;
    let Some(build) = redeemed.first().and_then(|r| r["build"].as_u64()) else {
        return answer(
            id,
            false,
            &json!("no archived, unredeemed release for this build and commit"),
        );
    };
    let fingerprint = crate::core::seal::fingerprint(&public);
    let said = format!(
        "release {build} (Buildkite {}) redeemed for key {fingerprint}",
        job.build
    );
    if let Ok(s) = super::platform::Settings::from(env) {
        let _ = super::events::append(env, &s, "release.apple.redeemed", None, None, &said, false)
            .await;
    }
    let info = crate::core::seal::info(apple::PIPELINE, job.build, &job.commit);
    let sealed = match grant(env).await {
        Ok(grant) => {
            let plain = serde_json::to_vec(&grant)?;
            crate::core::seal::seal(recipient, &info, &plain, &seed()?)
        }
        Err(why) => Err(why),
    };
    match sealed {
        Ok(sealed) => answer(id, true, &json!(sealed)),
        Err(why) => {
            d1(env, &[apple::fail(build, &why, now())]).await?;
            answer(id, false, &json!(format!("the grant failed: {why}")))
        }
    }
}

/// `release.apple.archive {offset}`: the signing release's archive in pieces, with its size and
/// R2's etag.
async fn archive_piece(env: &Env, job: &Job, id: &Value, args: &Value) -> Result<Response> {
    let rows = d1_rows(env, &crate::iosrelease::signing_row(job.build, &job.commit)).await?;
    let Some(build) = rows.first().and_then(|r| r["build"].as_u64()) else {
        return answer(
            id,
            false,
            &json!("no release of this build and commit is signing"),
        );
    };
    let offset = args["offset"]
        .as_u64()
        .or_else(|| args["offset"].as_str().and_then(|t| t.parse().ok()))
        .unwrap_or(0);
    let key = crate::iosrelease::key(&job.commit, build);
    let bucket = super::builds_bucket(env)?;
    let Some(head) = bucket.head(key.clone()).await? else {
        return answer(id, false, &json!("the archive is missing"));
    };
    let size = head.size();
    if offset > size {
        return answer(id, false, &json!("`offset` is past the end"));
    }
    let length = ARCHIVE_PIECE.min(size - offset);
    let bytes = if length == 0 {
        Vec::new()
    } else {
        let object = bucket
            .get(key)
            .range(worker::Range::OffsetWithLength { offset, length })
            .execute()
            .await?;
        match object.as_ref().and_then(worker::Object::body) {
            Some(body) => body.bytes().await?,
            None => return answer(id, false, &json!("the archive is missing")),
        }
    };
    let next = (offset + length < size).then_some(offset + length);
    answer(
        id,
        true,
        &json!({
            "data": crate::core::mirroring::base64url(&bytes),
            "next": next,
            "size": size,
            "etag": head.etag(),
        }),
    )
}
