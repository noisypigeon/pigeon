//! Memberships in the control plane, kept by the `Instance`; `GET /v0/app/members` answers a
//! device's signature; see `.claude/decisions/2026-11-14-security-instance-members-roles-invitations.md` §5.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::devices::DEVICES;
use super::elicit;
use super::platform::{self, Settings, now};
use crate::core::account::{self, Accounts};
use crate::core::app::devices::{Device, Devices};
use crate::core::member::join::{self, JoinRequest, Joins};
use crate::core::member::{self, Members, Role};

/// The route a device's signature lets through.
pub const PATH: &str = "/v0/app/members";

fn s(body: &Value, k: &str) -> String {
    body[k].as_str().unwrap_or_default().to_string()
}

fn fail(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!({ "error": why, "code": code }))?.with_status(code))
}

/// `/members/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    if matches!(
        path,
        "/members/join-challenge" | "/members/join" | "/members/joined"
    ) {
        return joining(state, path, &body).await;
    }
    if path == "/members/shadow" {
        return shadow(state, &body).await;
    }
    if path.starts_with("/members/bind/") {
        return binding(state, path, &body).await;
    }
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
        "/members/administrator" => Response::from_json(&json!({
            "administrator": member::shadow::administrator(&members) == Some(caller.account.as_str())
        })),
        "/members/view" => Response::from_json(&members.view(&caller.account)),
        "/members/invite" => {
            let (token, email) = (s(&body, "token"), s(&body, "email"));
            match members.invite(&token, &email, Role::Standard, &caller.account, t) {
                Ok((members, inv)) => {
                    storage.put(member::STORE, &members).await?;
                    Response::from_json(&json!({ "id": inv.id, "expires": inv.expires }))
                }
                Err(why) => fail(&why, 403),
            }
        }
        "/members/admit" => {
            let joins: Joins = storage.get(join::STORE).await?.unwrap_or_default();
            let id = s(&body, "id");
            match joins.admit(members, devices.clone(), &id, &caller.account, t) {
                Ok((joins, members, devices)) => {
                    storage.put(join::STORE, &joins).await?;
                    storage.put(member::STORE, &members).await?;
                    storage.put(DEVICES, &devices).await?;
                    Response::from_json(&json!({ "status": "admitted" }))
                }
                Err(why) => fail(&why, 403),
            }
        }
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

// A standalone's invitations, joins and admissions; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its 2026-11-15
// amendment, §7.

/// `/members/*` on the `Instance` for a join, which no device has signed yet.
async fn joining(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let members: Members = storage.get(member::STORE).await?.unwrap_or_default();
    let joins: Joins = storage.get(join::STORE).await?.unwrap_or_default();
    let t = now();
    match path {
        "/members/join-challenge" => {
            let random = platform::random_hex(32)?;
            let token = body["token"].as_str().unwrap_or_default();
            match joins.challenge(&members, token, random.as_bytes(), t) {
                Ok((joins, challenge)) => {
                    storage.put(join::STORE, &joins).await?;
                    Response::from_json(&json!({ "challenge": challenge }))
                }
                Err(why) => fail(&why, 404),
            }
        }
        "/members/join" => {
            let Ok(req) = serde_json::from_value::<JoinRequest>(body["join"].clone()) else {
                return fail("not a join", 400);
            };
            let accounts: Accounts = storage.get(account::STORE).await?.unwrap_or_default();
            let rp = body["rp"].as_str().unwrap_or_default();
            let (joins, r) = joins.join(
                members,
                accounts,
                &req,
                rp,
                rp,
                &platform::random_hex(16)?,
                t,
            );
            storage.put(join::STORE, &joins).await?;
            match r {
                Ok((members, accounts, inv)) => {
                    storage.put(member::STORE, &members).await?;
                    storage.put(account::STORE, &accounts).await?;
                    Response::from_json(
                        &json!({ "id": inv.id, "match_code": inv.match_code, "status": "enrolled" }),
                    )
                }
                Err(why) => fail(&why, 403),
            }
        }
        "/members/joined" => {
            let id = body["id"].as_str().unwrap_or_default();
            match Joins::state(&members, id, t) {
                Some(s) => Response::from_json(&json!({ "status": s })),
                None => fail("no such invitation", 404),
            }
        }
        _ => fail("not found", 404),
    }
}

/// `POST /v0/app/join/challenge` and `POST /v0/app/join`, before any device is linked, and
/// `GET /v0/app/join/<id>`.
///
/// # Errors
///
/// When a binding fails.
pub async fn join_route(
    env: &Env,
    s: &Settings,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    let rp = super::app::relying_party(env);
    let (op, arg) = match (method, path) {
        (Method::Post, "/v0/app/join/challenge") => {
            ("/members/join-challenge", json!({ "token": body["token"] }))
        }
        (Method::Post, "/v0/app/join") => ("/members/join", json!({ "join": body, "rp": rp })),
        (Method::Get, p) if p.starts_with("/v0/app/join/") => (
            "/members/joined",
            json!({ "id": &p["/v0/app/join/".len()..] }),
        ),
        _ => return Response::error("not found", 404),
    };
    let mut v = elicit::inbox(env, s, op, &arg).await?;
    if op == "/members/join-challenge" && v["error"].is_null() {
        v["rp"] = json!(rp);
    }
    answer(&v)
}

fn answer(v: &Value) -> Result<Response> {
    match v["error"].as_str() {
        Some(why) => {
            let code = v["code"].as_u64().and_then(|c| u16::try_from(c).ok());
            Response::error(why, code.unwrap_or(500))
        }
        None => Response::from_json(&v),
    }
}

/// `POST /v0/app/challenge` on a standalone: a passkey challenge for `member:invite` (`id` is
/// the email) or `member:admit` (`id` is the invitation).
///
/// # Errors
///
/// When a binding fails.
pub async fn challenge(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let id = body["id"].as_str().unwrap_or_default();
    let purpose = match body["op"].as_str() {
        Some("member:invite") if !id.is_empty() => join::invite_purpose(id),
        Some("member:admit") if !id.is_empty() => join::admit_purpose(id),
        _ => return Response::error("not a member challenge", 400),
    };
    let mut v = elicit::inbox(
        env,
        s,
        "/passkeys/challenge",
        &json!({ "purpose": purpose }),
    )
    .await?;
    v["rp"] = json!(super::app::relying_party(env));
    Response::from_json(&v)
}

/// `POST /v0/app/members/invites` (`{email, passkey}`) and `POST /v0/app/members/admit`
/// (`{id, passkey}`), each proved by an administrator's passkey.
///
/// # Errors
///
/// When a binding fails.
pub async fn write(
    env: &Env,
    s: &Settings,
    device: &Device,
    base: &str,
    path: &str,
    body: &Value,
) -> Result<Response> {
    let form = crate::core::app::approvals::proof_form(body);
    let rp = super::app::relying_party(env);
    let field = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let (purpose, op, arg) = match path {
        "/v0/app/members/invites" => {
            let email = field("email");
            let token = platform::random_hex(32)?;
            let arg = json!({ "device": device.id, "email": email, "token": token });
            (join::invite_purpose(&email), "/members/invite", arg)
        }
        "/v0/app/members/admit" => {
            let id = field("id");
            (
                join::admit_purpose(&id),
                "/members/admit",
                json!({ "device": device.id, "id": id }),
            )
        }
        _ => return Response::error("not found", 404),
    };
    if let Err(why) = super::passkeys::prove(env, s, &form, &purpose, &rp).await? {
        return Response::error(format!("refused: {why}"), 403);
    }
    let mut v = elicit::inbox(env, s, op, &arg).await?;
    if op == "/members/invite" && v["error"].is_null() {
        let token = arg["token"].as_str().unwrap_or_default();
        v["token"] = json!(token);
        v["link"] = json!(format!("pigeon://join?base={base}&token={token}"));
    }
    answer(&v)
}

/// Where each difference R3's shadow has logged is kept, by its key.
const SHADOWED: &str = "member.shadow";

/// How many differences are kept before new ones stop being logged.
const SHADOWED_MAX: usize = 2000;

/// `/members/shadow` on the `Instance`: `check` beside today's decision, answering a difference
/// not logged before; see the feed-view ADR's R3 amendment.
async fn shadow(state: &State, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let stored: Members = storage.get(member::STORE).await?.unwrap_or_default();
    let t = now();
    let members = stored.clone().seed_services(t);
    if members != stored {
        storage.put(member::STORE, &members).await?;
    }
    let (Ok(roles), Ok(caller)) = (
        super::standalone::roles(),
        serde_json::from_value::<member::shadow::Caller>(body["caller"].clone()),
    ) else {
        return Response::from_json(&json!({ "shadow": null }));
    };
    let Some(op) = crate::core::wire::catalog()
        .iter()
        .find(|o| Some(o.name) == body["operation"].as_str())
    else {
        return Response::from_json(&json!({ "shadow": null }));
    };
    let args = body["args"].as_object().cloned().unwrap_or_default();
    let today = body["today"].as_bool() == Some(true);
    let Some(diff) = member::shadow::compare(&roles, &members, &caller, op, &args, today, t) else {
        return Response::from_json(&json!({ "shadow": null }));
    };
    let mut seen: std::collections::BTreeSet<String> =
        storage.get(SHADOWED).await?.unwrap_or_default();
    if seen.len() >= SHADOWED_MAX || !seen.insert(diff.key()) {
        return Response::from_json(&json!({ "shadow": null }));
    }
    storage.put(SHADOWED, &seen).await?;
    Response::from_json(&json!({ "shadow": diff }))
}

/// Where the `Instance` keeps `service.bind`'s offers.
const BIND_OFFERS: &str = "members.bind";

fn refused(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!(why))?.with_status(code))
}

/// `/members/bind/offer`, `/peek` and `/grant`: `service.bind`'s offers, and the binding the
/// Administrator's passkey grants; see
/// `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`, its step 9
/// amendments.
async fn binding(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut offers: member::bind::Offers = storage.get(BIND_OFFERS).await?.unwrap_or_default();
    let t = now();
    let token = s(body, "token");
    match path {
        "/members/bind/offer" => {
            let expires = body["expires"].as_u64().unwrap_or(t);
            match offers.offer(
                &s(body, "account"),
                &s(body, "client_hash"),
                &token,
                t,
                expires,
            ) {
                Ok(o) => {
                    storage.put(BIND_OFFERS, &offers).await?;
                    Response::from_json(&o)
                }
                Err(why) => refused(&why, 400),
            }
        }
        "/members/bind/peek" => match offers.peek(&token, t) {
            Ok(o) => Response::from_json(o),
            Err(why) => refused(&why, 403),
        },
        "/members/bind/grant" => {
            let offer = match offers.take(&token, t) {
                Ok(o) => o,
                Err(why) => return refused(&why, 403),
            };
            storage.put(BIND_OFFERS, &offers).await?;
            let members: Members = storage.get(member::STORE).await?.unwrap_or_default();
            let members = members.seed_services(t);
            let Some(by) = member::shadow::administrator(&members).map(str::to_string) else {
                return refused("no administrator", 409);
            };
            let done = if offer.unbind {
                members.unbind_hash(&offer.client_hash, &by, t)
            } else {
                members
                    .bind_hash(&offer.account, &offer.client_hash, &by, t)
                    .map(|(m, _)| m)
            };
            match done {
                Ok(members) => {
                    storage.put(member::STORE, &members).await?;
                    Response::from_json(&offer)
                }
                Err(why) => refused(&why, 409),
            }
        }
        _ => refused("not found", 404),
    }
}
