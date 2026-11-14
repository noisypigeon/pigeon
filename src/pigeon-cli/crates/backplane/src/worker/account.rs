//! Accounts and profiles in the control plane: the `Instance` keeps them; `/v0/app/profile` and
//! its siblings answer a device's signature; see
//! `.claude/decisions/2026-11-14-runtime-accounts-profiles-instances.md`.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::devices::DEVICES;
use super::elicit;
use super::platform::{self, Settings, now};
use crate::core::account::{self, Accounts};
use crate::core::app::approvals;
use crate::devices::{Device, Devices};
use crate::push::{b64url, b64url_decode};

const PROFILE: &str = "/v0/app/profile";
const AVATAR: &str = "/v0/app/profile/avatar";
const DEVICE_PREFIX: &str = "/v0/app/devices/";
const PASSKEY_PREFIX: &str = "/v0/app/passkeys/";

// The `Instance` side.

fn fail(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!({ "error": why, "code": code }))?.with_status(code))
}

/// `/accounts/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let accounts: Accounts = storage.get(account::STORE).await?.unwrap_or_default();
    let devices: Devices = storage.get(DEVICES).await?.unwrap_or_default();
    let t = now();
    let (accounts, owner) = accounts.ensure_owner(&platform::random_hex(16)?, t);
    let mut devices = account::stamp(devices, &owner.id);
    let id = body["device"].as_str().unwrap_or_default().to_string();
    let Some(caller) = devices.devices.iter().find(|d| d.id == id).cloned() else {
        storage.put(account::STORE, &accounts).await?;
        storage.put(DEVICES, &devices).await?;
        return fail("no such device", 404);
    };
    let acct = caller.account.clone();
    let s = |k: &str| body[k].as_str().map(str::to_string);
    let (accounts, response) = match path {
        "/accounts/me" => {
            let list = listed(&devices, &acct, &id);
            let record = accounts.accounts.iter().find(|a| a.id == acct);
            let answer = json!({
                "account": record, "profile": accounts.profile(&acct), "devices": list,
            });
            (accounts, Ok(answer))
        }
        "/accounts/update" => {
            match accounts.clone().update(
                &acct,
                s("name").as_deref(),
                s("appearance").as_deref(),
                t,
            ) {
                Ok((next, p)) => (next, Ok(json!({ "profile": p }))),
                Err(why) => (accounts, Err((why, 400))),
            }
        }
        "/accounts/avatar/put" => {
            let jpeg = b64url_decode(&s("jpeg").unwrap_or_default());
            match jpeg {
                Err(_) => (
                    accounts,
                    Err(("the avatar isn't base64url".to_string(), 400)),
                ),
                Ok(bytes) => match accounts.clone().set_avatar(&acct, &bytes, t) {
                    Ok((next, p)) => {
                        storage
                            .put(&account::avatar_key(&acct), b64url(&bytes))
                            .await?;
                        (next, Ok(json!({ "profile": p })))
                    }
                    Err(why) => (accounts, Err((why, 400))),
                },
            }
        }
        "/accounts/avatar/get" => {
            let jpeg: Option<String> = storage.get(&account::avatar_key(&acct)).await?;
            let answer = jpeg.map_or(Err(("no avatar".to_string(), 404)), |j| {
                Ok(json!({ "jpeg": j }))
            });
            (accounts, answer)
        }
        "/accounts/avatar/delete" => match accounts.clone().clear_avatar(&acct, t) {
            Ok((next, p)) => {
                storage.delete(&account::avatar_key(&acct)).await?;
                (next, Ok(json!({ "profile": p })))
            }
            Err(why) => (accounts, Err((why, 400))),
        },
        "/accounts/unlink" => {
            let target = s("id").unwrap_or_default();
            if devices
                .devices
                .iter()
                .any(|d| d.id == target && d.account == acct)
            {
                devices = devices.unlink(&target);
                (
                    accounts,
                    Ok(json!({ "status": "ok", "signed_out": target == id })),
                )
            } else {
                (accounts, Err(("no such device".to_string(), 404)))
            }
        }
        _ => (accounts, Err(("not found".to_string(), 404))),
    };
    storage.put(account::STORE, &accounts).await?;
    storage.put(DEVICES, &devices).await?;
    match response {
        Ok(v) => Response::from_json(&v),
        Err((why, code)) => fail(&why, code),
    }
}

fn listed(devices: &Devices, acct: &str, current: &str) -> Vec<Value> {
    devices
        .devices
        .iter()
        .filter(|d| d.account == acct)
        .map(|d| {
            json!({
                "id": d.id, "name": d.name, "linked": d.linked, "seen": d.seen,
                "current": d.id == current,
            })
        })
        .collect()
}

// The public side.

/// Whether `path` is one this surface serves.
#[must_use]
pub fn handles(path: &str) -> bool {
    path == PROFILE
        || path == AVATAR
        || path.starts_with(DEVICE_PREFIX)
        || path.starts_with(PASSKEY_PREFIX)
}

/// `v` as a response: its `error` with its `code`, else itself.
fn answer(v: &Value) -> Result<Response> {
    match v["error"].as_str() {
        Some(why) => {
            let code = v["code"].as_u64().and_then(|c| u16::try_from(c).ok());
            Response::error(why, code.unwrap_or(500))
        }
        None => Response::from_json(v),
    }
}

async fn instance(
    env: &Env,
    s: &Settings,
    path: &str,
    device: &Device,
    extra: Value,
) -> Result<Value> {
    let mut body = extra;
    body["device"] = json!(device.id);
    elicit::inbox(env, s, path, &body).await
}

/// The routes a device's signature has let through that this surface serves.
///
/// # Errors
///
/// When a binding fails.
pub async fn route(
    env: &Env,
    s: &Settings,
    device: &Device,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    match (method, path) {
        (Method::Get, PROFILE) => {
            let mut v = instance(env, s, "/accounts/me", device, json!({})).await?;
            if v["error"].is_null() {
                let keys = elicit::inbox(env, s, "/passkeys/list", &json!({})).await?;
                v["passkeys"] = keys["keys"].clone();
            }
            answer(&v)
        }
        (Method::Patch, PROFILE) => {
            let extra = json!({ "name": body["name"], "appearance": body["appearance"] });
            answer(&instance(env, s, "/accounts/update", device, extra).await?)
        }
        (Method::Get, AVATAR) => {
            let v = instance(env, s, "/accounts/avatar/get", device, json!({})).await?;
            let bytes = v["jpeg"].as_str().and_then(|j| b64url_decode(j).ok());
            match bytes {
                Some(b) => {
                    let mut r = Response::from_bytes(b)?;
                    r.headers_mut().set("content-type", "image/jpeg")?;
                    Ok(r)
                }
                None => answer(&v),
            }
        }
        (Method::Put | Method::Post, AVATAR) => {
            let extra = json!({ "jpeg": body["jpeg"] });
            answer(&instance(env, s, "/accounts/avatar/put", device, extra).await?)
        }
        (Method::Delete, AVATAR) => {
            answer(&instance(env, s, "/accounts/avatar/delete", device, json!({})).await?)
        }
        (Method::Delete, p) if p.starts_with(DEVICE_PREFIX) => {
            let id = p.trim_start_matches(DEVICE_PREFIX);
            if let Err(why) = proved(env, s, body, &account::unlink_purpose(id)).await? {
                return Response::error(format!("refused: {why}"), 403);
            }
            answer(&instance(env, s, "/accounts/unlink", device, json!({ "id": id })).await?)
        }
        (Method::Delete, p) if p.starts_with(PASSKEY_PREFIX) => {
            let id = p.trim_start_matches(PASSKEY_PREFIX);
            if let Err(why) = proved(env, s, body, &account::passkey_remove_purpose(id)).await? {
                return Response::error(format!("refused: {why}"), 403);
            }
            let keys = elicit::inbox(env, s, "/passkeys/list", &json!({})).await?;
            let list = keys["keys"].as_array().cloned().unwrap_or_default();
            if !list.iter().any(|k| k["id"] == id) {
                return Response::error("no such passkey", 404);
            }
            if let Err(why) = account::may_remove_passkey(list.len()) {
                return Response::error(why, 409);
            }
            elicit::inbox(env, s, "/passkeys/remove", &json!({ "id": id })).await?;
            Response::from_json(&json!({ "status": "ok" }))
        }
        _ => Response::error("not found", 404),
    }
}

/// The owner's passkey answer in `body` for `purpose`.
async fn proved(
    env: &Env,
    s: &Settings,
    body: &Value,
    purpose: &str,
) -> Result<std::result::Result<(), String>> {
    let form = approvals::proof_form(body);
    let rp = super::app::relying_party(env);
    super::passkeys::prove(env, s, &form, purpose, &rp).await
}
