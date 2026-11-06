//! `/approve/<number>`: the page a grant's link opens, and the POST that uses it as the
//! signed-in person; see the amendment of 2026-11-06 to
//! `.claude/decisions/2026-11-06-development-control-plane-approvals-images-state.md` §1.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result};

use super::platform::Settings;
use super::{run, to_change};
use crate::access::Identity;
use crate::approve;
use crate::changes::Change;
use crate::grants::Grant;
use crate::mcp::Link;
use crate::wire;

fn html(status: u16, body: &str) -> Result<Response> {
    let mut r = Response::from_html(body)?.with_status(status);
    let h = r.headers_mut();
    h.set("content-security-policy", approve::CSP)?;
    h.set("cache-control", "no-store")?;
    h.set("referrer-policy", "same-origin")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

fn refused(status: u16, why: &str) -> Result<Response> {
    html(status, &approve::outcome("Not done", why))
}

/// The change number in `/approve/<number>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/approve/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Mints a grant of `operation` on change `number`, and the link offering it on `host`.
///
/// # Errors
///
/// A message when the change refuses it, or a binding fails.
pub async fn offer(
    env: &Env,
    s: &Settings,
    host: &str,
    number: u32,
    operation: &str,
) -> std::result::Result<Link, String> {
    let body = json!({ "operation": operation });
    let (status, v) = to_change(env, &s.repo, number, Method::Post, "/grant", Some(&body))
        .await
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(v.as_str().map_or_else(|| v.to_string(), str::to_string));
    }
    let token = v["token"].as_str().ok_or("the grant has no token")?;
    let grant: Grant = serde_json::from_value(v["grant"].clone()).map_err(|e| e.to_string())?;
    let head = grant
        .head
        .as_deref()
        .map(|h| format!(" at {h}"))
        .unwrap_or_default();
    Ok(Link {
        url: format!("https://{host}/approve/{number}?grant={token}"),
        message: format!(
            "`{operation}` on change {number}, {}{head}: open the link, check it, and confirm.",
            v["title"].as_str().unwrap_or_default()
        ),
        id: crate::grants::key(token)[..16].to_string(),
    })
}

/// Serves `/approve/<number>` for `who`.
///
/// # Errors
///
/// When a binding fails.
pub async fn serve(mut req: Request, env: &Env, s: &Settings, who: &Identity) -> Result<Response> {
    let Some(n) = number(&req.path()) else {
        return refused(404, "not found");
    };
    if !who.human {
        return refused(403, "only a person signed in to Access approves");
    }
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
            html(200, &approve::page(&change, &grant, &token))
        }
        Method::Post => {
            let origin = req.headers().get("origin")?;
            let site = req.headers().get("sec-fetch-site")?;
            if !approve::same_origin(origin.as_deref(), site.as_deref(), &host) {
                return refused(403, "this form is posted only from its own page");
            }
            let form = req.text().await?;
            let (Some(token), Some(operation)) = (
                approve::field(&form, "grant"),
                approve::field(&form, "operation"),
            ) else {
                return refused(400, "the form is missing its grant");
            };
            let body = json!({ "token": token, "operation": operation });
            let (status, v) =
                to_change(env, &s.repo, n, Method::Post, "/redeem", Some(&body)).await?;
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
        _ => refused(405, "method not allowed"),
    }
}
