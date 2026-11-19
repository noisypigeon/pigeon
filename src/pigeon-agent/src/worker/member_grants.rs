//! `member.grant` and `member.revoke`, and `/member-grant/<id>`, the page where the
//! Administrator grants a Standard member a capability, or revokes it, with a passkey; see
//! `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`, its "rest in
//! parallel" amendment.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::member::grant::{self, GRANT, Offer, REVOKE};

fn text<'a>(args: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Runs `member.grant` or `member.revoke`: an offer waiting for the Administrator's passkey,
/// answered with its link.
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
    let (Some(account), Some(capability)) = (text(args, "account"), text(args, "capability"))
    else {
        return Ok((400, json!("`account` and `capability` name the grant")));
    };
    let revoke = match op {
        GRANT => false,
        REVOKE => true,
        _ => return Ok((404, json!(format!("`{op}` is not a grant operation")))),
    };
    let draft = Offer {
        id: 0,
        account: account.into(),
        capability: capability.into(),
        scope: if revoke {
            None
        } else {
            text(args, "scope").map(str::to_string)
        },
        until: if revoke {
            None
        } else {
            args.get("until").and_then(Value::as_u64)
        },
        revoke,
        expires: 0,
    };
    let token = platform::random_hex(32)?;
    let expires = now() + crate::core::approval::grants::PAGE_TTL;
    let body = json!({ "draft": draft, "token": token, "expires": expires });
    let (status, v) = to_instance(env, s, "/members/grant/offer", &body).await?;
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
    if grant::parse(path).is_none() {
        return Ok(false);
    }
    let (status, _) =
        to_instance(env, s, "/members/grant/peek", &json!({ "token": token })).await?;
    Ok(status == 200)
}

/// The offer `token` names, for the proof's view.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn peek(env: &Env, s: &Settings, token: &str) -> Result<Option<Offer>> {
    let (status, v) =
        to_instance(env, s, "/members/grant/peek", &json!({ "token": token })).await?;
    Ok((status == 200)
        .then(|| serde_json::from_value(v).ok())
        .flatten())
}

/// Serves `/member-grant/<id>` for `who`: the page, and the POST that grants with the passkey.
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
        return page::refused(403, "only the Administrator grants a capability");
    }
    let path = req.path();
    let Some(id) = grant::parse(&path) else {
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
                    &approve::member_grant_page(&o, &token),
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
            let shown = format!("{}{id}?grant={token}", grant::PAGE);
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) =
                to_instance(env, s, "/members/grant/grant", &json!({ "token": token })).await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            let Ok(offer) = serde_json::from_value::<Offer>(v.clone()) else {
                return page::refused(409, v.as_str().unwrap_or("nothing was granted"));
            };
            if status != 200 {
                return page::refused(409, v.as_str().unwrap_or("nothing was granted"));
            }
            let kind = if offer.revoke {
                "member.revoked"
            } else {
                "member.granted"
            };
            let said = offer.title();
            let _ = super::events::append(env, s, kind, None, None, &said, false).await;
            page::html(200, &approve::outcome("Done", &said))
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix(grant::PAGE),
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
