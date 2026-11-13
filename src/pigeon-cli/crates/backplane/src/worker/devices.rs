//! The app's devices in the control plane: the `Instance` keeps them; `/app/link` links one with
//! the owner's proof; `/v0/app/*` takes a device's signature in place of Access; see
//! `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §2-§4.

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::approve::{html, html_with, refused};
use super::elicit;
use super::platform::{self, Settings, now, random_hex};
use crate::access::Identity;
use crate::approve as page;
use crate::devices::{self, Device, Devices};
use crate::push::{b64url, b64url_decode};

/// Where the `Instance` keeps the devices.
const DEVICES: &str = "devices";

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
            let (next, r) = d.check(&s("header"), &s("method"), &s("path"), &body, t);
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

/// The routes a device's signature has let through.
async fn signed(
    env: &Env,
    s: &Settings,
    req: &Request,
    device: &Device,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    const APPROVALS: &str = "/v0/app/approvals/";
    match (method, path) {
        (Method::Get, "/v0/app/whoami") => {
            Response::from_json(&json!({ "device": device.id, "name": device.name }))
        }
        (Method::Put, "/v0/app/device/push") => {
            let token = body["token"].as_str().unwrap_or_default();
            if !crate::core::app::push::token_ok(token) {
                return Response::error("not an APNs device token", 400);
            }
            to_instance(
                env,
                s,
                "/devices/push",
                &json!({ "id": device.id, "token": token }),
            )
            .await?;
            Response::from_json(&json!({ "status": "ok" }))
        }
        (Method::Get, "/v0/app/home") => {
            let mut home = super::insights::home(env).await?;
            home["approvals"] = super::app::list(env, s).await?;
            home["auto"] = super::app::auto_state(env, s).await?;
            Response::from_json(&home)
        }
        (Method::Get, "/v0/app/approvals") => Response::from_json(&super::app::list(env, s).await?),
        (Method::Get, p) if p.starts_with(APPROVALS) => {
            super::app::detail(env, s, p.trim_start_matches(APPROVALS)).await
        }
        (Method::Post, "/v0/app/challenge") => super::app::challenge(env, s, body).await,
        (Method::Post, p) if p.starts_with(APPROVALS) => {
            super::app::approve(env, s, p.trim_start_matches(APPROVALS), body).await
        }
        (Method::Post, "/v0/app/auto") => super::app::auto_start(env, s, body).await,
        (Method::Delete, "/v0/app/auto") => super::app::auto_end(env, s, body).await,
        (Method::Get, "/v0/app/insights") => {
            let url = req.url()?;
            let q = |k: &str| {
                url.query_pairs()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            };
            let range = q("range").parse().unwrap_or(1);
            match super::insights::detail(env, &q("metric"), &q("source"), range).await? {
                Ok(v) => Response::from_json(&v),
                Err(why) => Response::error(why, 400),
            }
        }
        _ => Response::error("not found", 404),
    }
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
            if let Err(why) = prove(env, s, &form, &purpose, &host).await? {
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
            if let Err(why) = prove(env, s, &form, &format!("device:{id}"), &host).await? {
                return refused(403, &why);
            }
            to_instance(env, s, "/devices/remove", &json!({ "id": id })).await?;
            list(env, s, Some("Unlinked.")).await
        }
        _ => refused(404, "not found"),
    }
}

/// The owner's proof on a page: a passkey answer for `purpose`, or the code.
async fn prove(
    env: &Env,
    s: &Settings,
    form: &str,
    purpose: &str,
    host: &str,
) -> Result<std::result::Result<(), String>> {
    if super::passkeys::answered(form) {
        super::passkeys::prove(env, s, form, purpose, host).await
    } else {
        let code = page::field(form, "code").unwrap_or_default();
        elicit::check_code(env, s, code.trim()).await
    }
}

async fn list(env: &Env, s: &Settings, said: Option<&str>) -> Result<Response> {
    let v = to_instance(env, s, "/devices/list", &json!({})).await?;
    let devices: Vec<Device> = serde_json::from_value(v["devices"].clone()).unwrap_or_default();
    let rows: Vec<(String, String, u64, u64)> = devices
        .into_iter()
        .map(|d| (d.id, d.name, d.linked, d.seen))
        .collect();
    html(200, &page::devices_page(&rows, said))
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
