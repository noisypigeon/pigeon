//! `/approve/<number>` and `/approvals`: the page a pending approval's link opens, its inbox, and
//! the POST that approves with the owner's code as the signed-in person; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result};

use super::platform::{Settings, now};
use super::{elicit, run, to_change};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::changes::Change;
use crate::grants::Grant;
use crate::wire;

pub(super) fn html(status: u16, body: &str) -> Result<Response> {
    html_with(status, body, approve::CSP)
}

pub(super) fn html_with(status: u16, body: &str, csp: &str) -> Result<Response> {
    let mut r = Response::from_html(body)?.with_status(status);
    let h = r.headers_mut();
    h.set("content-security-policy", csp)?;
    h.set("cache-control", "no-store")?;
    h.set("referrer-policy", "same-origin")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

pub(super) fn refused(status: u16, why: &str) -> Result<Response> {
    html(status, &approve::outcome("Not done", why))
}

/// The change number in `/approve/<number>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/approve/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Mints a page grant of `operation` on change `number`, puts it in the inbox, and answers the
/// link to its page on `host`, and when it expires.
///
/// # Errors
///
/// A message when the change refuses it, or a binding fails.
pub async fn pending(
    env: &Env,
    s: &Settings,
    host: &str,
    number: u32,
    operation: &str,
) -> std::result::Result<(String, u64), String> {
    let body = json!({ "operation": operation, "page": true });
    let (status, v) = to_change(env, &s.repo, number, Method::Post, "/grant", Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(v.as_str().map_or_else(|| v.to_string(), str::to_string));
    }
    let token = v["token"].as_str().ok_or("the grant has no token")?;
    let grant: Grant = serde_json::from_value(v["grant"].clone()).map_err(|e| e.to_string())?;
    let path = format!("/approve/{number}?grant={token}");
    let waiting = Waiting {
        number,
        operation: operation.to_string(),
        title: v["title"].as_str().unwrap_or_default().to_string(),
        path: path.clone(),
        expires: grant.expires,
    };
    elicit::inbox(env, s, "/approvals/add", &json!(waiting))
        .await
        .map_err(|e| e.to_string())?;
    Ok((format!("https://{host}{path}"), grant.expires))
}

/// The approvals still waiting: those whose link is unexpired and unused, the rest dropped
/// from the inbox.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn live(env: &Env, s: &Settings) -> Result<Vec<Waiting>> {
    let v = elicit::inbox(env, s, "/approvals/list", &json!({})).await?;
    let list: Vec<Waiting> = serde_json::from_value(v).unwrap_or_default();
    let mut kept = Vec::new();
    for w in list {
        let token = w.path.split_once("?grant=").map(|(_, t)| t.to_string());
        let alive = match (token, w.expires > now()) {
            (Some(token), true) => {
                let body = json!({ "token": token });
                if w.path.starts_with("/platform/") {
                    super::platforms::live(env, s, w.number, &body).await?
                } else if w.path.starts_with("/mandate/") {
                    super::mandates::to_instance(env, s, "/mandates/peek", &body)
                        .await?
                        .0
                        == 200
                } else {
                    let (status, v) =
                        to_change(env, &s.repo, w.number, Method::Post, "/peek", Some(&body))
                            .await?;
                    status == 200
                        && match (
                            serde_json::from_value::<Change>(v["change"].clone()),
                            serde_json::from_value::<Grant>(v["grant"].clone()),
                        ) {
                            (Ok(c), Ok(g)) => crate::grants::still_applies(&c, &g),
                            _ => false,
                        }
                }
            }
            _ => false,
        };
        if alive {
            kept.push(w);
        } else {
            elicit::inbox(env, s, "/approvals/remove", &json!({ "path": w.path })).await?;
        }
    }
    Ok(kept)
}

/// Serves `/approvals` and `/approve/<number>` for `who`.
///
/// # Errors
///
/// When a binding fails.
pub async fn serve(req: Request, env: &Env, s: &Settings, who: &Identity) -> Result<Response> {
    if !who.human {
        return refused(403, "only a person signed in to Access approves");
    }
    let path = req.path();
    if path == "/approvals" {
        let list = live(env, s).await?;
        let digest = match super::push::build(env, s).await {
            Ok((d, _)) => d.lines(),
            Err(e) => vec![format!("the digest is unavailable: {e}")],
        };
        return html_with(
            200,
            &approve::inbox_with(&list, &digest),
            approve::INBOX_CSP,
        );
    }
    let Some(n) = number(&path) else {
        return refused(404, "not found");
    };
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    match req.method() {
        Method::Get => {
            let token = url.query().and_then(|q| approve::field(q, "grant"));
            let Some(token) = token else {
                return refused(400, "this link has no grant");
            };
            let body = json!({ "token": token });
            let (status, v) =
                to_change(env, &s.repo, n, Method::Post, "/peek", Some(&body)).await?;
            if status != 200 {
                return refused(403, v.as_str().unwrap_or("this link is not valid"));
            }
            let (Ok(change), Ok(grant)) = (
                serde_json::from_value::<Change>(v["change"].clone()),
                serde_json::from_value::<Grant>(v["grant"].clone()),
            ) else {
                return refused(500, "the change's answer was unreadable");
            };
            html_with(
                200,
                &approve::page(&change, &grant, &token),
                approve::PROOF_CSP,
            )
        }
        Method::Post => post(req, env, s, who, n, &host).await,
        _ => refused(405, "method not allowed"),
    }
}

/// `POST /approve/<number>`: the owner's code when the grant needs it, then the operation as
/// `who`.
async fn post(
    mut req: Request,
    env: &Env,
    s: &Settings,
    who: &Identity,
    n: u32,
    host: &str,
) -> Result<Response> {
    let origin = req.headers().get("origin")?;
    let site = req.headers().get("sec-fetch-site")?;
    if !approve::same_origin(origin.as_deref(), site.as_deref(), host) {
        return refused(403, "this form is posted only from its own page");
    }
    let form = req.text().await?;
    let (Some(token), Some(operation)) = (
        approve::field(&form, "grant"),
        approve::field(&form, "operation"),
    ) else {
        return refused(400, "the form is missing its grant");
    };
    let (status, v) = to_change(
        env,
        &s.repo,
        n,
        Method::Post,
        "/peek",
        Some(&json!({ "token": token })),
    )
    .await?;
    if status != 200 {
        return refused(403, v.as_str().unwrap_or("this link is not valid"));
    }
    let Ok(peeked) = serde_json::from_value::<Grant>(v["grant"].clone()) else {
        return refused(500, "the grant was unreadable");
    };
    if peeked.needs_code() {
        let proved = if super::passkeys::answered(&form) {
            let purpose = crate::passkeys::grant_purpose(&token);
            super::passkeys::prove(env, s, &form, &purpose, host).await?
        } else {
            let code = approve::field(&form, "code").unwrap_or_default();
            elicit::check_code(env, s, code.trim()).await?
        };
        if let Err(why) = proved {
            return refused(403, &why);
        }
    }
    let review = approve::field(&form, "review").as_deref() == Some("1");
    let body =
        json!({ "token": token, "operation": operation, "by": who.subject, "review": review });
    let (status, v) = to_change(env, &s.repo, n, Method::Post, "/redeem", Some(&body)).await?;
    let _ = elicit::inbox(
        env,
        s,
        "/approvals/remove",
        &json!({ "path": format!("/approve/{n}?grant={token}") }),
    )
    .await;
    if status != 200 {
        return refused(403, v.as_str().unwrap_or("this link is not valid"));
    }
    let Ok(grant) = serde_json::from_value::<Grant>(v) else {
        return refused(500, "the grant was unreadable");
    };
    let mut args = Map::new();
    args.insert("number".into(), Value::from(n));
    if grant.operation == "change.approve"
        && let Some(head) = &grant.head
    {
        args.insert("sha".into(), Value::from(head.clone()));
    }
    let request = wire::Request {
        operation: grant.operation.clone(),
        args,
    };
    match run(env, s, who, &request).await {
        Ok(r) if r.ok => html(
            200,
            &approve::outcome(
                &format!("Done: {} on change {n}", grant.operation),
                &format!("Recorded as {}.", who.subject),
            ),
        ),
        Ok(r) => refused(409, &r.violations.join("\n")),
        Err(e) => refused(409, &e.to_string()),
    }
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::any(crate::routes::Path::Prefix("/approve/"), serve_route),
    super::Route::any(crate::routes::Path::Exact("/approvals"), serve_route),
];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(serve(req, env, s, who))
}
