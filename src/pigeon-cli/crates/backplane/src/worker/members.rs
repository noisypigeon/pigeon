//! Memberships in the control plane, kept by the `Instance`; `GET /v0/app/members` answers a
//! device's signature; see `.claude/decisions/2026-11-14-security-instance-members-roles-invitations.md` §5.

use serde_json::{Value, json};
use worker::{Env, Response, Result, State};

use super::devices::DEVICES;
use super::elicit;
use super::platform::{self, Settings, now};
use crate::core::account::{self, Accounts};
use crate::core::member::{self, Members};
use crate::devices::{Device, Devices};

/// The route a device's signature lets through.
pub const PATH: &str = "/v0/app/members";

fn fail(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!({ "error": why, "code": code }))?.with_status(code))
}

/// `/members/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let accounts: Accounts = storage.get(account::STORE).await?.unwrap_or_default();
    let devices: Devices = storage.get(DEVICES).await?.unwrap_or_default();
    let members: Members = storage.get(member::STORE).await?.unwrap_or_default();
    let t = now();
    let (accounts, owner) = accounts.ensure_owner(&platform::random_hex(16)?, t);
    let devices = account::stamp(devices, &owner.id);
    let members = members.seed(&owner.id, t);
    storage.put(account::STORE, &accounts).await?;
    storage.put(DEVICES, &devices).await?;
    storage.put(member::STORE, &members).await?;
    let id = body["device"].as_str().unwrap_or_default();
    let Some(caller) = devices.devices.iter().find(|d| d.id == id) else {
        return fail("no such device", 404);
    };
    match path {
        "/members/view" => Response::from_json(&members.view(&caller.account)),
        _ => fail("not found", 404),
    }
}

/// `GET /v0/app/members`: the calling device's view of the members.
///
/// # Errors
///
/// When a binding fails.
pub async fn route(env: &Env, s: &Settings, device: &Device) -> Result<Response> {
    let answer = elicit::inbox(env, s, "/members/view", &json!({ "device": device.id })).await?;
    match answer["error"].as_str() {
        Some(why) => {
            let code = answer["code"].as_u64().and_then(|c| u16::try_from(c).ok());
            Response::error(why, code.unwrap_or(500))
        }
        None => Response::from_json(&answer),
    }
}
