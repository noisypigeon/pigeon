//! The owner's CI window in the control plane: `ci.window`, the `Instance` object's store of it,
//! and `/ci-window/<id>`, the page that grants it with the owner's code; see
//! `.claude/decisions/2026-11-10-development-code-checks-github-actions.md`, its 2026-11-11
//! amendment.

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::ci_window::{CiWindow, OPERATION, Offer, Window};
use crate::control;

/// The storage key of the window and its offers.
const KEY: &str = "ci-window";

/// `ci.window` for `hours`: an offer waiting for the owner's code, answered with its link.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(env: &Env, s: &Settings, hours: u32) -> Result<(u16, Value)> {
    let token = platform::random_hex(32)?;
    let expires = now() + crate::grants::PAGE_TTL;
    let body = json!({ "hours": hours, "token": token, "expires": expires });
    let (status, v) = to_instance(env, s, "/ci-window/offer", &body).await?;
    if status != 200 {
        return Ok((status, v));
    }
    let offer: Offer = serde_json::from_value(v)?;
    let path = format!("/ci-window/{}?grant={token}", offer.id);
    let title = if hours == 0 {
        "Close the CI window".to_string()
    } else {
        format!("Code checks on Cloudflare for {hours} hours")
    };
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
        .var(crate::cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((
        202,
        json!({ "offer": offer, "pending": { "operation": OPERATION, "link": format!("https://{host}{path}") } }),
    ))
}

/// The window, when it is open now.
///
/// # Errors
///
/// When a binding fails.
pub async fn current(env: &Env, s: &Settings) -> Result<Option<Window>> {
    let (_, v) = to_instance(env, s, "/ci-window/get", &json!({})).await?;
    let all: CiWindow = serde_json::from_value(v).unwrap_or_default();
    Ok(all.current(now()).cloned())
}

/// The `Instance` Durable Object's side: `/ci-window/get`, `offer`, `peek` and `grant`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: CiWindow = storage.get(KEY).await?.unwrap_or_default();
    let token = body["token"].as_str().unwrap_or_default();
    let answer = match path {
        "/ci-window/get" => return Response::from_json(&all),
        "/ci-window/peek" => {
            return match all.peek(token, now()) {
                Ok(o) => Response::from_json(o),
                Err(why) => Response::error(why, 403),
            };
        }
        "/ci-window/offer" => {
            let hours = body["hours"]
                .as_u64()
                .and_then(|h| u32::try_from(h).ok())
                .unwrap_or_default();
            let expires = body["expires"].as_u64().unwrap_or_default();
            Ok(json!(all.offer(hours, token, now(), expires)))
        }
        "/ci-window/grant" => all
            .grant(token, body["by"].as_str().unwrap_or_default(), now())
            .map(|w| json!({ "window": w })),
        _ => return Response::error("not found", 404),
    };
    storage.put(KEY, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}

/// Runs the break-glass on each open change with a code check queued or running on GitHub;
/// the changes moved.
async fn move_open(env: &Env, s: &Settings) -> Result<Vec<u32>> {
    let rows = platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?;
    let mut moved = Vec::new();
    for o in control::open_rows(&rows) {
        let (status, v) =
            super::to_change(env, &s.repo, o.number, Method::Post, "/window", None).await?;
        if status == 200 && v["moved"].as_bool() == Some(true) {
            moved.push(o.number);
        }
    }
    Ok(moved)
}

/// The id in `/ci-window/<id>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/ci-window/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Serves `/ci-window/<id>` for `who`: the page, and the POST that grants it with the code.
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
        return page::refused(403, "only a person signed in to Access opens the CI window");
    }
    let Some(id) = number(&req.path()) else {
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
                to_instance(env, s, "/ci-window/peek", &json!({ "token": token })).await?;
            match serde_json::from_value::<Offer>(v.clone()) {
                Ok(o) if status == 200 && o.id == id => {
                    page::html(200, &approve::ci_window_page(&o, &token))
                }
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
            let code = approve::field(&form, "code").unwrap_or_default();
            if let Err(why) = elicit::check_code(env, s, code.trim()).await? {
                return page::refused(403, &why);
            }
            let body = json!({ "token": token, "by": who.subject });
            let (status, v) = to_instance(env, s, "/ci-window/grant", &body).await?;
            let _ = elicit::inbox(
                env,
                s,
                "/approvals/remove",
                &json!({ "path": format!("/ci-window/{id}?grant={token}") }),
            )
            .await;
            if status != 200 {
                return page::refused(409, v.as_str().unwrap_or("the window was not changed"));
            }
            let (title, said) = match serde_json::from_value::<Window>(v["window"].clone()) {
                Ok(w) => {
                    let moved = move_open(env, s).await?;
                    let said = format!(
                        "the CI window is open until {} (Unix seconds), opened by {}: code checks run on the Checker; moved {moved:?}",
                        w.until, w.by
                    );
                    ("CI window open", said)
                }
                Err(_) => (
                    "CI window closed",
                    format!(
                        "the CI window was closed by {}: code checks go back to GitHub",
                        who.subject
                    ),
                ),
            };
            let _ = super::events::append(env, s, OPERATION, None, None, &said, false).await;
            page::html(200, &approve::outcome(title, &said))
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix("/ci-window/"),
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
