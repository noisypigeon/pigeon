//! `zones.set`, `.show` and `.list`, and `zones.private` on the `Instance`; see
//! `.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md` §1.

use serde_json::{Map, Value, json};
use worker::{Env, Response, Result, State};

use super::elicit;
use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use crate::approve::Waiting;
use crate::core::zones_private::{self as zp, LIST, Offer, Offers, SET, SHOW, Store};

/// The `Instance` paths of the private zone store.
pub(super) const PREFIX: &str = "/zones/private/";

fn text<'a>(args: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn refused(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!(why))?.with_status(code))
}

/// Runs `zones.set`, `zones.show` or `zones.list`: a set is an offer waiting for the
/// Administrator's passkey, answered with its link; the others read the store.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    op: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    match op {
        SET => {
            let alias = text(args, "alias").unwrap_or_default();
            if let Err(why) = zp::alias(alias) {
                return Ok((400, json!(why)));
            }
            let entry = match zp::entry(args.get("entry").unwrap_or(&Value::Null)) {
                Ok(e) => e,
                Err(why) => return Ok((400, json!(why))),
            };
            let draft = Offer {
                id: 0,
                alias: alias.into(),
                entry,
                expires: 0,
            };
            let token = platform::random_hex(32)?;
            let expires = now() + crate::core::approval::grants::PAGE_TTL;
            let body = json!({ "draft": draft, "token": token, "expires": expires });
            let (status, v) = to_instance(env, s, "/zones/private/offer", &body).await?;
            if status != 200 {
                return Ok((status, v));
            }
            let offer: Offer = serde_json::from_value(v)?;
            let path = offer.path(&token);
            let title = offer.title();
            let waiting = Waiting {
                number: offer.id,
                operation: op.into(),
                title: title.clone(),
                path: path.clone(),
                expires,
            };
            elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await?;
            let said = format!("`{op}` ({title}) waits for the Administrator");
            let _ =
                super::events::append(env, s, "approval.pending", None, None, &said, false).await;
            super::push::notify(env, s, "Approval waiting", &said, &path).await;
            let host = env
                .var(crate::wire::ship_wire::HOST_VAR)
                .map(|v| v.to_string())
                .unwrap_or_default();
            Ok((
                202,
                json!({
                    "offer": { "id": offer.id, "alias": offer.alias, "expires": offer.expires },
                    "pending": { "operation": op, "link": super::app_link(&host, &path) },
                }),
            ))
        }
        SHOW => {
            let alias = text(args, "alias").unwrap_or_default();
            if let Err(why) = zp::alias(alias) {
                return Ok((400, json!(why)));
            }
            to_instance(env, s, "/zones/private/show", &json!({ "alias": alias })).await
        }
        LIST => to_instance(env, s, "/zones/private/list", &json!({})).await,
        _ => Ok((
            404,
            json!(format!("`{op}` is not a private zone operation")),
        )),
    }
}

/// Redeems the offer `token` names, once the Administrator's passkey proves it, and logs that
/// the alias was set.
///
/// # Errors
///
/// When a binding fails.
#[expect(
    dead_code,
    reason = "the passkey page that redeems an offer is the gate change"
)]
pub(super) async fn redeem(env: &Env, s: &Settings, token: &str) -> Result<(u16, Value)> {
    let (status, v) = to_instance(env, s, "/zones/private/set", &json!({ "token": token })).await?;
    if status == 200 {
        let alias = v["alias"].as_str().unwrap_or_default();
        let said = format!("private zone `{alias}` set");
        let _ = super::events::append(env, s, zp::UPDATED, None, None, &said, false).await;
    }
    Ok((status, v))
}

/// `/zones/private/*` on the `Instance`: the offers, and the store the passkey's redemption
/// sets.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut offers: Offers = storage.get(zp::OFFERS).await?.unwrap_or_default();
    let t = now();
    let token = body["token"].as_str().unwrap_or_default().to_string();
    match path {
        "/zones/private/offer" => {
            let Ok(draft) = serde_json::from_value::<Offer>(body["draft"].clone()) else {
                return refused("the offer is malformed", 400);
            };
            let expires = body["expires"].as_u64().unwrap_or(t);
            match offers.offer(draft, &token, t, expires) {
                Ok(o) => {
                    storage.put(zp::OFFERS, &offers).await?;
                    Response::from_json(&o)
                }
                Err(why) => refused(&why, 400),
            }
        }
        "/zones/private/peek" => match offers.peek(&token, t) {
            Ok(o) => Response::from_json(o),
            Err(why) => refused(&why, 403),
        },
        "/zones/private/set" => {
            let offer = match offers.take(&token, t) {
                Ok(o) => o,
                Err(why) => return refused(&why, 403),
            };
            storage.put(zp::OFFERS, &offers).await?;
            let mut store: Store = storage.get(zp::STORE).await?.unwrap_or_default();
            zp::set(&mut store, &offer, t);
            storage.put(zp::STORE, &store).await?;
            Response::from_json(&json!({ "alias": offer.alias }))
        }
        "/zones/private/show" => {
            let store: Store = storage.get(zp::STORE).await?.unwrap_or_default();
            let alias = body["alias"].as_str().unwrap_or_default();
            match store.get(alias) {
                Some(e) => Response::from_json(&json!({
                    "alias": alias,
                    "zone": e.zone,
                    "services": e.services,
                    "records": e.records,
                    "updated": e.updated,
                })),
                None => refused(&format!("no private zone `{alias}`"), 404),
            }
        }
        "/zones/private/list" => {
            let store: Store = storage.get(zp::STORE).await?.unwrap_or_default();
            Response::from_json(&zp::listing(&store))
        }
        _ => refused("not found", 404),
    }
}
