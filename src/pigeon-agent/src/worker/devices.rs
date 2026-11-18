//! The app's devices in the control plane: the `Instance` keeps them; `/app/link` links one with
//! the owner's proof; `/v0/app/*` takes a device's signature in place of Access; see
//! `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §2-§4.

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::approve::{html_with, refused};
use super::elicit;
use super::platform::{self, Settings, now, random_hex};
use crate::access::Identity;
use crate::approve as page;
use crate::core::app::devices::{self, Device, Devices};
use crate::push::{b64url, b64url_decode};

/// Where the `Instance` keeps the devices.
pub(super) const DEVICES: &str = "devices";

/// The app's API, served before Access.
pub const API: &str = "/v0/app/";

/// Apple's association file, served before Access.
pub const ASSOCIATION: &str = "/.well-known/apple-app-site-association";

/// The app's bundle ID.
const BUNDLE_ID: &str = "dev.pigeon.app";

// The `Instance` side.

/// `/devices/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let d: Devices = storage.get(DEVICES).await?.unwrap_or_default();
    let t = now();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let (d, answer) = match path {
        "/devices/offer" => match d.clone().offer(&s("name"), &s("key"), &s("code"), t) {
            Ok(next) => (next, json!({ "status": "ok" })),
            Err(why) => (d, refusal(&why)),
        },
        "/devices/redeem" => {
            let (next, r) = d.redeem(&s("code"), &s("key"), &s("signature"), t);
            (next, device(r))
        }
        "/devices/check" => {
            let body = b64url_decode(&s("body")).unwrap_or_default();
            let (next, r) = d.check_attempt(
                &s("header"),
                &s("method"),
                &s("path"),
                &body,
                &s("attempt"),
                t,
            );
            (next, device(r))
        }
        "/devices/list" => {
            let d = d.prune(t);
            let list = json!({ "status": "ok", "devices": d.devices });
            (d, list)
        }
        "/devices/remove" => (d.unlink(&s("id")), json!({ "status": "ok" })),
        "/devices/push" => {
            let token = Some(s("token")).filter(|x| !x.is_empty());
            (d.set_push(&s("id"), token), json!({ "status": "ok" }))
        }
        _ => return Response::error("not found", 404),
    };
    storage.put(DEVICES, &d).await?;
    Response::from_json(&answer)
}

fn refusal(why: &str) -> Value {
    json!({ "status": "refused", "why": why })
}

fn device(r: std::result::Result<Device, String>) -> Value {
    match r {
        Ok(d) => json!({ "status": "ok", "device": d }),
        Err(why) => refusal(&why),
    }
}

async fn to_instance(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<Value> {
    elicit::inbox(env, s, path, body).await
}

/// The device that signed `req`'s `X-Pigeon-Device` header over `body`, or why not.
///
/// # Errors
///
/// When a binding fails.
pub async fn authenticate(
    env: &Env,
    s: &Settings,
    req: &Request,
    body: &[u8],
) -> Result<std::result::Result<Device, String>> {
    let Some(header) = req.headers().get(devices::HEADER)? else {
        return Ok(Err(format!("no `{}` header", devices::HEADER)));
    };
    let v = to_instance(
        env,
        s,
        "/devices/check",
        &json!({
            "header": header, "method": req.method().to_string(), "path": req.path(),
            "body": b64url(body),
            // One per delivery, resent by `platform::call`'s retry; see the iOS ADR's 2026-11-16 amendment.
            "attempt": random_hex(16)?,
        }),
    )
    .await?;
    Ok(answered(&v))
}

fn answered(v: &Value) -> std::result::Result<Device, String> {
    if v["status"] == "ok" {
        serde_json::from_value(v["device"].clone()).map_err(|e| e.to_string())
    } else {
        Err(v["why"].as_str().unwrap_or("refused").to_string())
    }
}

// Before Access.

/// Serves the association file and `/v0/app/*`, which Access lets through; nothing for any
/// other path.
///
/// # Errors
///
/// When a binding fails.
pub async fn public(mut req: Request, env: &Env) -> Option<Result<Response>> {
    let path = req.path();
    if path == ASSOCIATION {
        return Some(association(env));
    }
    if !path.starts_with(API) {
        return None;
    }
    Some(
        async move {
            let s = Settings::from(env)?;
            let method = req.method();
            if (method.clone(), path.as_str()) == (Method::Post, "/v0/app/session") {
                return session(env, &s, &mut req).await;
            }
            let bytes = if method == Method::Get {
                Vec::new()
            } else {
                req.bytes().await?
            };
            let device = match authenticate(env, &s, &req, &bytes).await? {
                Ok(d) => d,
                Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
            };
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            signed(env, &s, &req, &device, &method, &path, &body).await
        }
        .await,
    )
}

/// `POST /v0/app/session`: redeems a link code for the device that signed it.
async fn session(env: &Env, s: &Settings, req: &mut Request) -> Result<Response> {
    let body: Value = req.json().await.unwrap_or(Value::Null);
    let f = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let v = to_instance(
        env,
        s,
        "/devices/redeem",
        &json!({ "code": f("code"), "key": f("key"), "signature": f("signature") }),
    )
    .await?;
    match answered(&v) {
        Ok(d) => Response::from_json(&json!({ "device": d.id, "name": d.name })),
        Err(why) => Response::error(format!("refused: {why}"), 403),
    }
}

/// The routes a device's signature has let through: the app's MCP door alone; every other path
/// answers 426, as the R10b and R8c amendments to
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` have it.
async fn signed(
    env: &Env,
    s: &Settings,
    req: &Request,
    device: &Device,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    if (method, path) == (&Method::Post, "/v0/app/mcp") {
        return app_mcp(env, s, req, device, body).await;
    }
    Response::error("Update the app", 426)
}

fn association(env: &Env) -> Result<Response> {
    let team = platform::secret(env, "APPLE_TEAM_ID")?;
    let mut r = Response::from_json(&json!({
        "webcredentials": { "apps": [format!("{}.{BUNDLE_ID}", team.trim())] },
    }))?;
    r.headers_mut().set("cache-control", "max-age=3600")?;
    Ok(r)
}

// The pages, behind Access.

/// `/app/link`'s policy: the proof pages' (`/passkey.js`), and its form may return to the app as
/// `pigeon://linked`; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §2.
const LINK_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; form-action 'self' pigeon:; frame-ancestors 'none'; base-uri 'none'";

/// Serves `/app/link` and `/devices` for `who`.
///
/// # Errors
///
/// When a binding fails.
pub async fn page_serve(
    mut req: Request,
    env: &Env,
    s: &Settings,
    who: &Identity,
) -> Result<Response> {
    if !who.human {
        return refused(403, "only a person signed in to Access links a device");
    }
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    if req.method() != Method::Get {
        let origin = req.headers().get("origin")?;
        let site = req.headers().get("sec-fetch-site")?;
        if !page::same_origin(origin.as_deref(), site.as_deref(), &host) {
            return refused(403, "this is posted only from its own page");
        }
    }
    match (req.method(), req.path().as_str()) {
        (Method::Get, "/app/link") => {
            let q = |k: &str| {
                url.query_pairs()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            };
            let key = q("key");
            match devices::key_point(&key) {
                Ok(p) => html_with(
                    200,
                    &page::link_page(
                        &devices::clean_name(&q("name")),
                        &key,
                        &devices::fingerprint(&p),
                    ),
                    LINK_CSP,
                ),
                Err(why) => refused(400, &why),
            }
        }
        (Method::Post, "/app/link") => {
            let form = req.text().await?;
            let field = |k: &str| page::field(&form, k).unwrap_or_default();
            let key = field("key");
            let Ok(point) = devices::key_point(&key) else {
                return refused(400, "the key isn't a P-256 key");
            };
            let purpose = format!("device:{}", devices::fingerprint(&point));
            let proved =
                super::passkeys::on_page(env, s, &form, &purpose, &host, "/app/link").await?;
            if let Err(why) = proved {
                return refused(403, &why);
            }
            let code = random_hex(16)?;
            let v = to_instance(
                env,
                s,
                "/devices/offer",
                &json!({ "name": field("name"), "key": key, "code": code }),
            )
            .await?;
            if v["status"] != "ok" {
                return refused(400, v["why"].as_str().unwrap_or("refused"));
            }
            let mut r = Response::empty()?.with_status(303);
            r.headers_mut()
                .set("location", &format!("pigeon://linked?code={code}"))?;
            r.headers_mut().set("cache-control", "no-store")?;
            Ok(r)
        }
        (Method::Get, "/devices") => list(env, s, None).await,
        (Method::Post, "/devices") => {
            let form = req.text().await?;
            let id = page::field(&form, "remove").unwrap_or_default();
            let purpose = format!("device:{id}");
            let proved =
                super::passkeys::on_page(env, s, &form, &purpose, &host, "/devices").await?;
            if let Err(why) = proved {
                return refused(403, &why);
            }
            to_instance(env, s, "/devices/remove", &json!({ "id": id })).await?;
            list(env, s, Some("Unlinked.")).await
        }
        _ => refused(404, "not found"),
    }
}

async fn list(env: &Env, s: &Settings, said: Option<&str>) -> Result<Response> {
    let v = to_instance(env, s, "/devices/list", &json!({})).await?;
    let devices: Vec<Device> = serde_json::from_value(v["devices"].clone()).unwrap_or_default();
    let rows: Vec<(String, String, u64, u64)> = devices
        .into_iter()
        .map(|d| (d.id, d.name, d.linked, d.seen))
        .collect();
    html_with(200, &page::devices_page(&rows, said), page::PROOF_CSP)
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::any(crate::routes::Path::Exact("/app/link"), serve_route),
    super::Route::any(crate::routes::Path::Exact("/devices"), serve_route),
];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
}

/// `POST /v0/app/mcp`: one MCP message, served by `/mcp`'s handler with `device`'s person as its
/// caller.
async fn app_mcp(
    env: &Env,
    s: &Settings,
    req: &Request,
    device: &Device,
    body: &Value,
) -> Result<Response> {
    let (_, v) = super::mandates::to_instance(
        env,
        s,
        "/members/administrator",
        &serde_json::json!({ "device": device.id }),
    )
    .await?;
    let who = Identity {
        subject: if device.account.is_empty() {
            format!("device:{}", device.id)
        } else {
            device.account.clone()
        },
        human: v["administrator"].as_bool() == Some(true),
        device: true,
    };
    let sid = req.headers().get(crate::mcp::SESSION_HEADER)?;
    let host = req.url()?.host_str().unwrap_or_default().to_string();
    let door = super::api::Door {
        name: super::api::DEVICE,
        person: true,
        host: &host,
        device: Some(device),
    };
    super::api::serve_message(env, s, &who, &door, sid.as_deref(), body).await
}
