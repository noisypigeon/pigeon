//! Forwards each request to the color the lease names; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its step 9a amendment.
#![cfg(feature = "worker")]

use std::cell::RefCell;

use worker::{Context, Env, Headers, Request, RequestInit, Response, Result, event};

/// The header naming the colour an answer was forwarded to, so a probe tells a host the front
/// answered from one its colour answered directly (the blue-green ADR's exact-host route amendment).
pub const FORWARDED: &str = "x-pigeon-front";

/// The header one colour's call to another carries; never taken from outside, so the front drops it
/// (the blue-green ADR's colour-calls amendment).
pub const COLOUR_TOKEN: &str = "x-pigeon-colour-token";

/// How long an isolate keeps the lease's color, in milliseconds.
const CACHE_MS: u64 = 5_000;

thread_local! {
    static LEASE: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// The binding a color is reached through: upper-cased, with `-` as `_`.
fn binding(color: &str) -> String {
    color.to_ascii_uppercase().replace('-', "_")
}

/// The lease's color, from D1 `DB`'s `SELECT color FROM lease WHERE id = 1`, cached for [`CACHE_MS`]; none without a row.
async fn color(env: &Env) -> Option<String> {
    let now = worker::Date::now().as_millis();
    if let Some(c) = LEASE.with(|l| {
        l.borrow()
            .as_ref()
            .filter(|(_, at)| now.saturating_sub(*at) < CACHE_MS)
            .map(|(c, _)| c.clone())
    }) {
        return Some(c);
    }
    let db = env.d1("DB").ok()?;
    let row = db
        .prepare("SELECT color FROM lease WHERE id = 1")
        .first::<serde_json::Value>(None)
        .await
        .ok()??;
    let c = row.get("color")?.as_str()?.to_string();
    LEASE.with(|l| *l.borrow_mut() = Some((c.clone(), now)));
    Some(c)
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let Some(c) = color(&env).await else {
        return Response::error("no lease", 503);
    };
    let Ok(service) = env.service(&binding(&c)) else {
        return Response::error(format!("no binding for {c}"), 503);
    };
    let req = if req.headers().has(COLOUR_TOKEN)? {
        without_colour_token(&req)?
    } else {
        req
    };
    let answer = service.fetch_request(req).await?;
    // A copy of its headers, marked: a forwarded answer's own headers can't be changed.
    let headers = Headers::new();
    for (k, v) in answer.headers().entries() {
        headers.append(&k, &v)?;
    }
    headers.set(FORWARDED, &c)?;
    Ok(answer.with_headers(headers))
}

/// `req` with every header but [`COLOUR_TOKEN`].
fn without_colour_token(req: &Request) -> Result<Request> {
    let headers = Headers::new();
    for (k, v) in req.headers().entries() {
        if !k.eq_ignore_ascii_case(COLOUR_TOKEN) {
            headers.append(&k, &v)?;
        }
    }
    let mut init = RequestInit::new();
    init.with_method(req.method()).with_headers(headers);
    if let Some(body) = req.inner().body() {
        init.with_body(Some(body.into()));
    }
    Request::new_with_init(req.url()?.as_str(), &init)
}
