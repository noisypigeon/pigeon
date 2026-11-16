//! An app-shape standalone: with `SHAPE=app` the Worker answers `/v0/app/*` alone, seeds its
//! `Instance` once from `HANDOVER`, and holds its roles to the release's; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`, its 2026-11-15
//! amendment.

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::devices::{self, DEVICES};
use super::elicit;
use super::passkeys::KEYRING;
use super::platform::{self, Settings, now};
use crate::core::account::{self, Accounts};
use crate::core::app::devices::{Device, Devices};
use crate::core::member::handover::{Handover, Seeded};
use crate::core::member::{self, Members, Roles, check_roles};

/// The release's roles, as a standalone holds them.
const ROLES: &str = include_str!("../../../../.config/roles.yaml");

/// Where the `Instance` notes that it was seeded.
const SEEDED: &str = "standalone.seeded";

/// Whether this Worker is an app-shape standalone.
#[must_use]
pub fn is_app(env: &Env) -> bool {
    env.var("SHAPE").is_ok_and(|v| v.to_string() == "app")
}

/// The roles a standalone serves under, or why it can't serve.
///
/// # Errors
///
/// When the compiled-in roles don't parse or fail [`check_roles`].
pub fn roles() -> std::result::Result<Roles, String> {
    let r = Roles::parse_file(ROLES)?;
    let bad = check_roles(&r);
    if bad.is_empty() {
        Ok(r)
    } else {
        Err(bad.join("; "))
    }
}

fn settings(env: &Env) -> Result<Settings> {
    let instance = env.var("INSTANCE_NAME")?.to_string();
    Ok(Settings {
        repo: instance.clone(),
        namespace: instance.clone(),
        instance,
        floor: 1,
        team: String::new(),
        aud: String::new(),
    })
}

/// The Worker's `fetch` on an app-shape standalone.
///
/// # Errors
///
/// When a binding fails.
pub async fn fetch(mut req: Request, env: &Env) -> Result<Response> {
    let path = req.path();
    if !path.starts_with(devices::API) {
        return Response::error("not found", 404);
    }
    if let Err(why) = roles() {
        return Response::error(format!("this instance's roles are unsound: {why}"), 500);
    }
    let s = settings(env)?;
    let handover = platform::secret(env, "HANDOVER").unwrap_or_default();
    let v = elicit::inbox(
        env,
        &s,
        "/standalone/seed",
        &json!({ "handover": handover }),
    )
    .await?;
    if let Some(why) = v["error"].as_str() {
        return Response::error(why.to_string(), 503);
    }
    let method = req.method();
    // Its own host under the platform's zone, when it has one; see
    // .claude/decisions/2026-11-15-runtime-standalone-hostnames-platform-zone.md §5.
    let own = env.secret("BASE_URL").map(|b| b.to_string()).ok();
    let base = match own.filter(|b| b.starts_with("https://")) {
        Some(b) => b,
        None => req.url()?.origin().ascii_serialization(),
    };
    let bytes = if method == Method::Get {
        Vec::new()
    } else {
        req.bytes().await?
    };
    // Joining runs before this device is linked; its passkey and signature prove it.
    if path == "/v0/app/join" || path.starts_with("/v0/app/join/") {
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return super::members::join_route(env, &s, &method, &path, &body).await;
    }
    let device = match devices::authenticate(env, &s, &req, &bytes).await? {
        Ok(d) => d,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    signed(env, &s, &device, &base, &method, &path, &body).await
}

/// The routes a device's signature lets through on a standalone.
async fn signed(
    env: &Env,
    s: &Settings,
    device: &Device,
    base: &str,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    match (method, path) {
        (Method::Get, "/v0/app/whoami") => {
            Response::from_json(&json!({ "device": device.id, "name": device.name }))
        }
        (Method::Get, super::members::PATH) => super::members::route(env, s, device).await,
        (Method::Get, "/v0/app/instances") => {
            let features: Vec<String> = env
                .var("FEATURES")
                .map(|v| v.to_string())
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect();
            Response::from_json(&json!({ "instances": [{
                "name": s.instance, "mode": "standalone", "shape": "app", "base_url": base,
                "features": features, "current": true,
            }] }))
        }
        (Method::Post, "/v0/app/challenge") => super::members::challenge(env, s, body).await,
        (Method::Post, p) if p.starts_with("/v0/app/members/") => {
            super::members::write(env, s, device, base, p, body).await
        }
        (m, p) if p.starts_with(super::learner::API) => {
            super::learner::route(env, s, device, m, p, body).await
        }
        _ => Response::error("not found", 404),
    }
}

// The `Instance` side.

/// `/standalone/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    match path {
        "/standalone/seed" => {
            if storage.get::<bool>(SEEDED).await?.unwrap_or_default() {
                return Response::from_json(&json!({ "seeded": false }));
            }
            let stores = Seeded {
                accounts: storage.get(account::STORE).await?.unwrap_or_default(),
                members: storage.get(member::STORE).await?.unwrap_or_default(),
                devices: storage.get(DEVICES).await?.unwrap_or_default(),
                ring: storage.get(KEYRING).await?.unwrap_or_default(),
            };
            if !stores.is_empty() {
                storage.put(SEEDED, true).await?;
                return Response::from_json(&json!({ "seeded": false }));
            }
            let h = match Handover::parse(body["handover"].as_str().unwrap_or_default()) {
                Ok(h) => h,
                Err(why) => return Response::from_json(&json!({ "error": why })),
            };
            let (s, _) = h.seed(stores, now());
            let (accounts, members, devices, ring): (Accounts, Members, Devices, _) =
                (s.accounts, s.members, s.devices, s.ring);
            storage.put(account::STORE, &accounts).await?;
            storage.put(member::STORE, &members).await?;
            storage.put(DEVICES, &devices).await?;
            storage.put(KEYRING, &ring).await?;
            storage.put(SEEDED, true).await?;
            Response::from_json(&json!({ "seeded": true }))
        }
        _ => Response::error("not found", 404),
    }
}
