//! `secrets.migrate`, and `/secrets-migrate/<id>`, the page where the owner approves, with a
//! passkey, copying shared Worker secrets into the Secrets Store in the Worker; see
//! `.claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md`, its `Shared secrets
//! move to the Secrets Store by an in-Worker copy` amendment.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::platform::migrate::{self, Copied, OPERATION, Offer, Offers};

/// The var naming the platform's Secrets Store.
const STORE_VAR: &str = "SECRETS_STORE";

/// Runs `secrets.migrate`: an offer waiting for the owner's passkey, answered with its link.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(env: &Env, s: &Settings, args: &Map<String, Value>) -> Result<(u16, Value)> {
    let names: Vec<String> = args
        .get("names")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Err(why) = migrate::names(&names) {
        return Ok((400, json!(why)));
    }
    let token = platform::random_hex(32)?;
    let expires = now() + crate::core::approval::grants::PAGE_TTL;
    let body = json!({ "names": names, "token": token, "expires": expires });
    let (status, v) = to_instance(env, s, "/secrets/migrate/offer", &body).await?;
    if status != 200 {
        return Ok((status, v));
    }
    let offer: Offer = serde_json::from_value(v)?;
    let path = offer.path(&token);
    let title = offer.title();
    let waiting = Waiting {
        number: offer.id,
        operation: OPERATION.into(),
        title: title.clone(),
        path: path.clone(),
        expires,
    };
    elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await?;
    let said = format!("`{OPERATION}` ({title}) waits for the owner");
    let _ = super::events::append(env, s, "approval.pending", None, None, &said, false).await;
    super::push::notify(env, s, "Approval waiting", &said, &path).await;
    let host = env
        .var(crate::wire::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((
        202,
        json!({ "offer": offer, "pending": { "operation": OPERATION, "link": super::app_link(&host, &path) } }),
    ))
}

/// Whether the offer behind inbox entry `path` is still live.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn live(env: &Env, s: &Settings, path: &str, token: &str) -> Result<bool> {
    if migrate::parse(path).is_none() {
        return Ok(false);
    }
    let (status, _) =
        to_instance(env, s, "/secrets/migrate/peek", &json!({ "token": token })).await?;
    Ok(status == 200)
}

/// The `Instance`'s side: `/secrets/migrate/offer`, `/peek` and `/take`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let refused = |why: &str, code: u16| -> Result<Response> {
        Ok(Response::from_json(&json!(why))?.with_status(code))
    };
    let storage = state.storage();
    let offers: Offers = storage.get(migrate::STORE).await?.unwrap_or_default();
    let t = now();
    let token = body["token"].as_str().unwrap_or_default();
    match path {
        "/secrets/migrate/offer" => {
            let names: Vec<String> = body["names"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let expires = body["expires"].as_u64().unwrap_or(t);
            match offers.offer(&names, token, t, expires) {
                Ok((offers, o)) => {
                    storage.put(migrate::STORE, &offers).await?;
                    Response::from_json(&o)
                }
                Err(why) => refused(&why, 400),
            }
        }
        "/secrets/migrate/peek" => match offers.peek(token, t) {
            Ok(o) => Response::from_json(o),
            Err(why) => refused(&why, 403),
        },
        "/secrets/migrate/take" => match offers.take(token, t) {
            Ok((offers, o)) => {
                storage.put(migrate::STORE, &offers).await?;
                Response::from_json(&o)
            }
            Err(why) => refused(&why, 403),
        },
        _ => refused("not found", 404),
    }
}

/// Copies each of `names` from the Worker's own secrets into the store, through one token minted
/// for the call and revoked after, whatever happened. No value is put in what it answers.
async fn copy(env: &Env, names: &[String]) -> Result<std::result::Result<Vec<Copied>, String>> {
    let Ok(store) = env.var(STORE_VAR).map(|v| v.to_string()) else {
        return Ok(Err(format!("the Worker has no `{STORE_VAR}` var")));
    };
    let key = match env.secret("SESSION_KEY") {
        Ok(k) => k.to_string(),
        Err(_) => return Ok(Err("the Worker has no `SESSION_KEY` secret".into())),
    };
    let (account, id, token) =
        match super::minting::mint_helper(env, migrate::PERMISSION, migrate::TOKEN_TTL).await? {
            Ok(t) => t,
            Err(why) => return Ok(Err(why)),
        };
    let path = format!("/accounts/{account}/secrets_store/stores/{store}/secrets");
    let mut copied = Vec::new();
    for name in names {
        let store_name = migrate::store_name(name);
        // The Worker secret itself, never `platform::secret`'s store fallback.
        let Some(value) = env.secret(name).ok().map(|v| v.to_string()) else {
            copied.push(Copied {
                name: name.clone(),
                store: store_name,
                outcome: "missing".into(),
                fingerprint: None,
            });
            continue;
        };
        let fingerprint = Some(migrate::fingerprint(&key, &value));
        let body = json!([{
            "name": store_name,
            "value": value,
            "scopes": ["workers"],
            "comment": "copied by secrets.migrate",
        }]);
        let outcome = match platform::cloudflare(&token, Method::Post, &path, Some(&body)).await {
            Ok(_) => "written".to_string(),
            Err(e) if e.to_string().contains("already exists") => {
                "exists: the store kept its own value, which this fingerprint may not match"
                    .to_string()
            }
            Err(e) => {
                let why = e.to_string().replace(&value, "…");
                format!("failed: create: {why}")
            }
        };
        copied.push(Copied {
            name: name.clone(),
            store: store_name,
            outcome,
            fingerprint,
        });
    }
    if let Ok(Err(why)) | Err(why) = super::minting::revoke_token(env, &id)
        .await
        .map_err(|e| e.to_string())
    {
        copied.push(Copied {
            name: "(token)".into(),
            store: String::new(),
            outcome: format!("failed: revoke: {why}"),
            fingerprint: None,
        });
    }
    Ok(Ok(copied))
}

/// Serves `/secrets-migrate/<id>` for `who`: the page, and the POST that copies with the passkey.
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
        return page::refused(403, "only the owner copies secrets, never a workload");
    }
    let path = req.path();
    let Some(id) = migrate::parse(&path) else {
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
                to_instance(env, s, "/secrets/migrate/peek", &json!({ "token": token })).await?;
            match (status, serde_json::from_value::<Offer>(v)) {
                (200, Ok(o)) if o.id == id => page::html_with(
                    200,
                    &approve::secrets_migrate_page(&o, &token),
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
            let shown = format!("{}{id}?grant={token}", migrate::PAGE);
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) =
                to_instance(env, s, "/secrets/migrate/take", &json!({ "token": token })).await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            let (200, Ok(offer)) = (status, serde_json::from_value::<Offer>(v.clone())) else {
                return page::refused(409, v.as_str().unwrap_or("the offer was not live"));
            };
            let copied = match copy(env, &offer.names).await? {
                Ok(c) => c,
                Err(why) => return page::refused(409, &why),
            };
            let said = migrate::summary(&copied);
            let _ = super::events::append(
                env,
                s,
                "secrets.migrated",
                None,
                None,
                &format!("{}: {said}", offer.title()),
                false,
            )
            .await;
            page::html(200, &approve::outcome("Done", &said))
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix(migrate::PAGE),
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
