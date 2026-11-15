//! A standalone's host under the platform's zone, proxied to its stack's origin; see
//! `.claude/decisions/2026-11-15-runtime-standalone-hostnames-platform-zone.md` §3.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{Value, json};
use worker::js_sys::Uint8Array;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, Response, Result};

use super::platform::{self, json_request, now};
use crate::core::platform::hosts;

/// The `Instance` operation answering a host's origin.
pub const ORIGIN: &str = "/hosts/origin";

/// The `Instance` storage key of the private listing (the learner ADR §2).
const LISTING: &str = "instances.private";

/// How long an isolate keeps an answer, misses included, in seconds.
const CACHE_SECS: u64 = 60;

thread_local! {
    static CACHE: RefCell<HashMap<String, (Option<String>, u64)>> = RefCell::new(HashMap::new());
}

/// The host the proxy answers for `req`, or `None` for the platform's own routes: only where
/// `INSTANCE_ZONE` is set, and never the platform's own hosts.
#[must_use]
pub fn ours(req: &Request, env: &Env) -> Option<String> {
    let zone = env.var(hosts::ZONE_VAR).ok()?.to_string();
    let host = req.url().ok()?.host_str()?.to_ascii_lowercase();
    let own: Vec<String> = ["PASSKEY_RP_ID", "PUBLIC_HOST"]
        .iter()
        .filter_map(|k| env.var(k).ok().map(|v| v.to_string()))
        .collect();
    let own: Vec<&str> = own.iter().map(String::as_str).collect();
    hosts::handled(&host, &zone, &own).then(|| host.trim_end_matches('.').to_string())
}

/// Answers `req` for `host`: forwarded to its standalone's origin on `/v0/app/*`, a bare 404
/// otherwise. Nothing here logs the host.
///
/// # Errors
///
/// When a binding fails.
pub async fn serve(mut req: Request, env: &Env, host: &str) -> Result<Response> {
    let url = req.url()?;
    if !hosts::forwarded(url.path()) {
        return Response::empty().map(|r| r.with_status(404));
    }
    let Some(origin) = origin(env, host).await? else {
        return Response::empty().map(|r| r.with_status(404));
    };
    let path = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let headers = Headers::new();
    for (k, v) in req.headers() {
        if hosts::passes(&k) {
            headers.set(&k, &v)?;
        }
    }
    let method = req.method();
    let mut init = RequestInit::new();
    if !matches!(method, Method::Get | Method::Head) {
        let body = req.bytes().await?;
        init.with_body(Some(Uint8Array::from(body.as_slice()).into()));
    }
    init.with_method(method).with_headers(headers);
    let out = Request::new_with_init(&hosts::target(&origin, &path), &init)?;
    Fetch::Request(out).send().await
}

/// `host`'s origin, from this isolate's cache or the `Instance`.
async fn origin(env: &Env, host: &str) -> Result<Option<String>> {
    let t = now();
    let cached = CACHE.with(|c| {
        c.borrow()
            .get(host)
            .filter(|(_, at)| t.saturating_sub(*at) < CACHE_SECS)
            .map(|(o, _)| o.clone())
    });
    if let Some(o) = cached {
        return Ok(o);
    }
    let instance = env.var("INSTANCE_NAME")?.to_string();
    let req = json_request(
        &format!("https://instance{ORIGIN}"),
        Method::Post,
        Some(&json!({ "host": host })),
        None,
    )?;
    let stub = env
        .durable_object("INSTANCE")?
        .id_from_name(&instance)?
        .get_stub()?;
    let v: Value = platform::call(env, &stub, req).await?.json().await?;
    let o = v["origin"].as_str().map(str::to_string);
    CACHE.with(|c| c.borrow_mut().insert(host.to_string(), (o.clone(), t)));
    Ok(o)
}

/// The `Instance`'s side of [`ORIGIN`]: `{origin}` for `body.host`, or `{}`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve_origin(state: &worker::State, body: &Value) -> Result<Response> {
    let listing: Value = state.storage().get(LISTING).await?.unwrap_or(Value::Null);
    let host = body["host"].as_str().unwrap_or_default();
    Response::from_json(&match hosts::origin_of(&listing, host) {
        Some(o) => json!({ "origin": o }),
        None => json!({}),
    })
}
