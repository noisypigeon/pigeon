//! `/docs/`: the architecture site from its bucket, behind a passkey; see
//! `.claude/decisions/2026-11-11-runtime-docs-under-platform-passkey.md` §2 and §3.

use worker::{Env, Method, Request, Response, Result};

use super::approve::{html_with, refused};
use super::passkeys;
use super::platform::{self, Settings, now};
use crate::access::Identity;
use crate::approve as page;
use crate::core::sites::view::{self, Target};

/// The site's bucket binding.
pub(super) const BINDING: &str = "DOCS";

async fn serve(mut req: Request, env: &Env, s: &Settings, who: &Identity) -> Result<Response> {
    if !who.human {
        return refused(403, "only a person signed in to Access views the docs");
    }
    let path = req.path();
    let Some(target) = view::target(&path) else {
        return refused(404, "not found");
    };
    if target == Target::Slash {
        let mut r = Response::empty()?.with_status(301);
        r.headers_mut().set("location", "/docs/")?;
        return Ok(r);
    }
    let key = platform::secret(env, "SESSION_KEY")?.into_bytes();
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    let gate = |said: Option<&str>, has: bool| {
        html_with(200, &view::gate(&path, has, said), page::PROOF_CSP)
    };
    if req.method() == Method::Post {
        let origin = req.headers().get("origin")?;
        let site = req.headers().get("sec-fetch-site")?;
        if !page::same_origin(origin.as_deref(), site.as_deref(), &host) {
            return refused(403, "this is posted only from its own page");
        }
        let form = req.text().await?;
        if let Err(why) = passkeys::prove(env, s, &form, view::PURPOSE, &host).await? {
            return gate(Some(&format!("Refused: {why}")), true);
        }
        let value = view::sign(&key, &who.subject, now() + view::TTL);
        let mut r = Response::empty()?.with_status(303);
        r.headers_mut().set("location", &path)?;
        r.headers_mut()
            .set("set-cookie", &view::set_cookie(&value))?;
        r.headers_mut().set("cache-control", "no-store")?;
        return Ok(r);
    }
    let cookie = req.headers().get("cookie")?.unwrap_or_default();
    let seen = view::cookie(&cookie).is_some_and(|v| view::valid(&key, v, &who.subject, now()));
    if !seen {
        return gate(None, passkeys::registered(env, s).await?);
    }
    let Target::Key(key) = target else {
        return refused(404, "not found");
    };
    let bucket = env.bucket(BINDING)?;
    let (object, status) = match bucket.get(&key).execute().await? {
        Some(o) => (o, 200),
        None => match bucket.get("404.html").execute().await? {
            Some(o) => (o, 404),
            None => return refused(404, "not found"),
        },
    };
    let name = if status == 404 {
        "404.html"
    } else {
        key.as_str()
    };
    let bytes = match object.body() {
        Some(b) => b.bytes().await?,
        None => Vec::new(),
    };
    let mut r = Response::from_bytes(bytes)?.with_status(status);
    let h = r.headers_mut();
    h.set("content-type", view::content_type(name))?;
    h.set("content-security-policy", view::CSP)?;
    h.set("cache-control", "private, no-store")?;
    h.set("referrer-policy", "same-origin")?;
    h.set("x-content-type-options", "nosniff")?;
    Ok(r)
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::any(crate::routes::Path::Exact(view::PREFIX), serve_route),
    super::Route::any(crate::routes::Path::Prefix("/docs/"), serve_route),
];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(serve(req, env, s, who))
}
