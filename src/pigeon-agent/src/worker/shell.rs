//! The web client's static shell at `/app`: it holds no data and renders the four views from
//! `/mcp`; see `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` step 14.

use futures_util::future::LocalBoxFuture;
use worker::{Env, Method, Request, Response, Result};

use super::approve::{html_with, refused};
use super::platform::Settings;
use crate::access::Identity;

const BODY: &str = include_str!("shell/index.html");
const CSS: &str = include_str!("shell/shell.css");

/// `GET /app`'s policy: [`crate::approve::PROOF_CSP`], plus the service worker and the manifest.
const SHELL_CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'; worker-src 'self'; manifest-src 'self'";

/// The shell's scripts: each path and its source.
const SCRIPTS: [(&str, &str); 4] = [
    ("/app/mcp.js", include_str!("shell/mcp.js")),
    ("/app/views.js", include_str!("shell/views.js")),
    ("/app/app.js", include_str!("shell/app.js")),
    ("/app/manifest.json", include_str!("shell/manifest.json")),
];

/// The shell's routes.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::only(Method::Get, crate::routes::Path::Exact("/app"), serve_route),
    super::Route::only(
        Method::Get,
        crate::routes::Path::Exact("/app/mcp.js"),
        serve_route,
    ),
    super::Route::only(
        Method::Get,
        crate::routes::Path::Exact("/app/views.js"),
        serve_route,
    ),
    super::Route::only(
        Method::Get,
        crate::routes::Path::Exact("/app/app.js"),
        serve_route,
    ),
    super::Route::only(
        Method::Get,
        crate::routes::Path::Exact("/app/manifest.json"),
        serve_route,
    ),
];

fn serve_route<'a>(
    req: Request,
    _env: &'a Env,
    _s: &'a Settings,
    who: &'a Identity,
) -> LocalBoxFuture<'a, Result<Response>> {
    Box::pin(async move { serve(&req.path(), who) })
}

/// `GET /app` and its scripts, for a person.
fn serve(path: &str, who: &Identity) -> Result<Response> {
    if !who.human {
        return refused(403, "the web client is a person's");
    }
    if path == "/app" {
        let head = format!(
            "<link rel=\"manifest\" href=\"/app/manifest.json\"><style>{CSS}</style><script src=\"/app/mcp.js\" defer></script><script src=\"/app/views.js\" defer></script><script src=\"/app/app.js\" defer></script>"
        );
        return html_with(
            200,
            &crate::core::look::page("pigeon", &head, BODY),
            SHELL_CSP,
        );
    }
    let Some((_, source)) = SCRIPTS.iter().find(|(p, _)| *p == path) else {
        return refused(404, "not found");
    };
    let mut r = Response::ok(*source)?;
    let h = r.headers_mut();
    let kind = if path == "/app/manifest.json" {
        "application/manifest+json"
    } else {
        "text/javascript; charset=utf-8"
    };
    h.set("content-type", kind)?;
    h.set("cache-control", "no-cache")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}
