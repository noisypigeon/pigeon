//! `pool.status`, `pool.pause` and `pool.resume`, and `/pool-resume/<pool>/<id>`, the page that
//! resumes a pool with the owner's passkey; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md` §8 and its
//! 2026-11-17 amendment "Step 3b3 lands in two changes before step 9".

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result};

use super::platform::{self, Settings, now};
use super::pool::to_pool;
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::pool::config::config;
use crate::core::pool::resume::{self, OPERATION, Offer};

fn text<'a>(args: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// The pools `name` selects: that one, or every configured pool.
fn pools(name: Option<&str>) -> std::result::Result<Vec<String>, String> {
    let c = config()?;
    match name {
        Some(n) if c.pool(n).is_some() => Ok(vec![n.to_string()]),
        Some(n) => Err(format!("`{n}` is not a configured pool")),
        None => Ok(c.order.clone()),
    }
}

/// Runs pool operation `op` for `who`.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    op: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    let names = match pools(text(args, "pool")) {
        Ok(n) => n,
        Err(why) => return Ok((400, json!(why))),
    };
    if op != "pool.status" && text(args, "pool").is_none() {
        return Ok((400, json!("`pool` names the pool")));
    }
    match op {
        "pool.status" => {
            let mut out = Map::new();
            for n in names {
                let (_, v) = to_pool(env, &n, "/status", &json!({})).await?;
                out.insert(n, v);
            }
            Ok((200, Value::Object(out)))
        }
        "pool.pause" => {
            let Some(why) = text(args, "why") else {
                return Ok((400, json!("`why` says why the pool pauses")));
            };
            let name = &names[0];
            let why = format!("{why} (by {})", who.subject);
            let body = json!({ "pool": name, "why": why });
            let (status, v) = to_pool(env, name, "/pause", &body).await?;
            if status == 200 {
                let said = format!("pool {name} paused: {why}");
                let _ = super::events::append_about(
                    env,
                    s,
                    "pool.paused",
                    None,
                    None,
                    &said,
                    false,
                    &format!("pool:{name}"),
                )
                .await;
            }
            Ok((status, v))
        }
        OPERATION => offer(env, s, &names[0]).await,
        _ => Ok((404, json!(format!("`{op}` is not a pool operation")))),
    }
}

/// `pool.resume`: an offer waiting for the owner's passkey, answered with its link.
async fn offer(env: &Env, s: &Settings, name: &str) -> Result<(u16, Value)> {
    let token = platform::random_hex(32)?;
    let expires = now() + crate::core::approval::grants::PAGE_TTL;
    let body = json!({ "pool": name, "token": token, "expires": expires });
    let (status, v) = to_pool(env, name, "/resume/offer", &body).await?;
    if status != 200 {
        return Ok((status, v));
    }
    let offer: Offer = serde_json::from_value(v)?;
    let path = offer.path(&token);
    let title = format!("Resume the {name} pool");
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
pub(super) async fn live(env: &Env, path: &str, token: &str) -> Result<bool> {
    let Some((name, _)) = resume::parse(path) else {
        return Ok(false);
    };
    let (status, _) = to_pool(env, name, "/resume/peek", &json!({ "token": token })).await?;
    Ok(status == 200)
}

/// Serves `/pool-resume/<pool>/<id>` for `who`: the page, and the POST that resumes the pool
/// with the passkey.
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
        return page::refused(403, "only a person signed in to Access resumes a pool");
    }
    let path = req.path();
    let Some((name, id)) = resume::parse(&path).map(|(n, i)| (n.to_string(), i)) else {
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
                to_pool(env, &name, "/resume/peek", &json!({ "token": token })).await?;
            match serde_json::from_value::<Offer>(v.clone()) {
                Ok(o) if status == 200 && o.id == id && o.pool == name => page::html_with(
                    200,
                    &approve::pool_resume_page(&o, &token),
                    approve::PROOF_CSP,
                ),
                _ => page::refused(403, v.as_str().unwrap_or("this link is not valid")),
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
            let shown = format!("{}{name}/{id}?grant={token}", resume::PAGE);
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) =
                to_pool(env, &name, "/resume/grant", &json!({ "token": token })).await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            if status != 200 {
                return page::refused(409, v.as_str().unwrap_or("the pool was not resumed"));
            }
            let by = super::account::approver(env, s).await?;
            let said = format!("pool {name} resumed by {by}");
            let _ = super::events::append_about(
                env,
                s,
                "pool.resumed",
                None,
                None,
                &said,
                false,
                &format!("pool:{name}"),
            )
            .await;
            page::html(200, &approve::outcome("Pool resumed", &said))
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix(resume::PAGE),
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
