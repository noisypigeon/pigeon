//! `lease.flip` and `promote.hold`, and `/lease-switch/<id>`, the page where the owner grants either with a passkey; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20 amendment.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, json_request, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::deploy::flip::{self, Offer, Offers, Switch};

/// Runs `lease.flip` or `promote.hold`: an offer waiting for the owner's passkey, answered with its link.
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
    let switch = match flip::switch(op, args) {
        Ok(sw) => sw,
        Err(why) => return Ok((400, json!(why))),
    };
    let token = platform::random_hex(32)?;
    let expires = now() + crate::core::approval::grants::PAGE_TTL;
    let body = json!({ "switch": switch, "token": token, "expires": expires });
    let (status, v) = to_instance(env, s, "/lease/switch/offer", &body).await?;
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
    let said = format!("`{op}` ({title}) waits for the owner");
    let _ = super::events::append(env, s, "approval.pending", None, None, &said, false).await;
    super::push::notify(env, s, "Approval waiting", &said, &path).await;
    let host = env
        .var(crate::wire::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((
        202,
        json!({ "offer": offer, "pending": { "operation": op, "link": super::app_link(&host, &path) } }),
    ))
}

/// Whether the offer behind inbox entry `path` is still live.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn live(env: &Env, s: &Settings, path: &str, token: &str) -> Result<bool> {
    if flip::parse(path).is_none() {
        return Ok(false);
    }
    let (status, _) = to_instance(env, s, "/lease/switch/peek", &json!({ "token": token })).await?;
    Ok(status == 200)
}

/// The `Instance`'s side: `/lease/switch/offer`, `/peek` and `/take`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let refused = |why: &str, code: u16| -> Result<Response> {
        Ok(Response::from_json(&json!(why))?.with_status(code))
    };
    let storage = state.storage();
    let offers: Offers = storage.get(flip::STORE).await?.unwrap_or_default();
    let t = now();
    let token = body["token"].as_str().unwrap_or_default();
    match path {
        "/lease/switch/offer" => {
            let Ok(switch) = serde_json::from_value::<Switch>(body["switch"].clone()) else {
                return refused("`switch` is a lease switch", 400);
            };
            let expires = body["expires"].as_u64().unwrap_or(t);
            let (offers, o) = offers.offer(switch, token, t, expires);
            storage.put(flip::STORE, &offers).await?;
            Response::from_json(&o)
        }
        "/lease/switch/peek" => match offers.peek(token, t) {
            Ok(o) => Response::from_json(o),
            Err(why) => refused(&why, 403),
        },
        "/lease/switch/take" => match offers.take(token, t) {
            Ok((offers, o)) => {
                storage.put(flip::STORE, &offers).await?;
                Response::from_json(&o)
            }
            Err(why) => refused(&why, 403),
        },
        _ => refused("not found", 404),
    }
}

/// Sends `switch` to `MainRun`: its status and answer.
async fn to_main(env: &Env, s: &Settings, switch: &Switch) -> Result<(u16, String)> {
    let (url, body) = match switch {
        Switch::Flip(c) => ("https://main/promote/flip", json!({ "color": c })),
        Switch::Hold(h) => ("https://main/promote/hold", json!({ "hold": h })),
    };
    let req = json_request(url, Method::Post, Some(&body), None)?;
    let mut r = platform::call(env, &super::main_stub(env, &s.repo)?, req).await?;
    Ok((r.status_code(), r.text().await.unwrap_or_default()))
}

/// Serves `/lease-switch/<id>` for `who`: the page, and the POST that switches with the passkey.
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
        return page::refused(403, "only the owner flips the lease or holds promotion");
    }
    let path = req.path();
    let Some(id) = flip::parse(&path) else {
        return page::refused(404, "not found");
    };
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    match req.method() {
        Method::Get => {
            let Some(token) = url.query().and_then(|q| approve::field(q, "grant")) else {
                return page::refused(400, "this link has no grant");
            };
            let (status, v) =
                to_instance(env, s, "/lease/switch/peek", &json!({ "token": token })).await?;
            match (status, serde_json::from_value::<Offer>(v)) {
                (200, Ok(o)) if o.id == id => page::html_with(
                    200,
                    &approve::lease_switch_page(&o, &token),
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
            let shown = format!("{}{id}?grant={token}", flip::PAGE);
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) =
                to_instance(env, s, "/lease/switch/take", &json!({ "token": token })).await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            let (200, Ok(offer)) = (status, serde_json::from_value::<Offer>(v.clone())) else {
                return page::refused(409, v.as_str().unwrap_or("the offer was not live"));
            };
            let (status, text) = to_main(env, s, &offer.switch).await?;
            if status != 200 {
                return page::refused(409, &format!("MainRun refused: {text}"));
            }
            let _ = super::events::append(
                env,
                s,
                "lease.switched",
                None,
                None,
                &format!("{}: {text}", offer.title()),
                false,
            )
            .await;
            page::html(200, &approve::outcome("Done", &text))
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix(flip::PAGE),
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
