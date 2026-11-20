//! `zones.set`, `.show` and `.list`, and `zones.private` on the `Instance`; see
//! `.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md` §1.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
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
async fn redeem(env: &Env, s: &Settings, token: &str) -> Result<(u16, Value)> {
    let (status, v) = to_instance(env, s, "/zones/private/set", &json!({ "token": token })).await?;
    if status == 200 {
        let alias = v["alias"].as_str().unwrap_or_default();
        let said = format!("private zone `{alias}` set");
        let _ = super::events::append(env, s, zp::UPDATED, None, None, &said, false).await;
    }
    Ok((status, v))
}

/// Whether the waiting set at `path` still holds its offer.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn live(env: &Env, s: &Settings, path: &str, token: &str) -> Result<bool> {
    if zp::parse(path).is_none() {
        return Ok(false);
    }
    Ok(peek(env, s, token).await?.is_some())
}

/// The offer `token` names, for the page and the proof's view.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn peek(env: &Env, s: &Settings, token: &str) -> Result<Option<Offer>> {
    let (status, v) =
        to_instance(env, s, "/zones/private/peek", &json!({ "token": token })).await?;
    Ok((status == 200)
        .then(|| serde_json::from_value(v).ok())
        .flatten())
}

/// Serves `/zone-private/<id>` for `who`: the page, and the POST that sets the zone with the
/// passkey.
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
        return page::refused(403, "only the Administrator sets a private zone");
    }
    let path = req.path();
    let Some(id) = zp::parse(&path) else {
        return page::refused(404, "not found");
    };
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    match req.method() {
        Method::Get => {
            let Some(token) = url.query().and_then(|q| approve::field(q, "grant")) else {
                return page::refused(400, "this link has no grant");
            };
            match peek(env, s, &token).await? {
                Some(o) if o.id == id => page::html_with(
                    200,
                    &approve::zone_private_page(&o, &token),
                    approve::PROOF_CSP,
                ),
                _ => page::refused(403, "this link is not valid; ask for a new one"),
            }
        }
        Method::Post => {
            let origin = req.headers().get("origin")?;
            let site = req.headers().get("sec-fetch-site")?;
            if !approve::same_origin(origin.as_deref(), site.as_deref(), &host) {
                return page::refused(403, "this form is posted only from its own page");
            }
            let form = req.text().await?;
            let Some(token) = approve::field(&form, "grant") else {
                return page::refused(400, "the form is missing its grant");
            };
            let purpose = crate::core::approval::passkeys::grant_purpose(&token);
            let shown = format!("{}{id}?grant={token}", zp::PAGE);
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) = redeem(env, s, &token).await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            if status != 200 {
                return page::refused(409, v.as_str().unwrap_or("nothing was set"));
            }
            let alias = v["alias"].as_str().unwrap_or_default();
            page::html(
                200,
                &approve::outcome("Done", &format!("private zone `{alias}` set")),
            )
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix(zp::PAGE),
    serve_route,
)];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
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
