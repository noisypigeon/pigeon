//! Forwards each request to the color the lease names; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its step 9a amendment.
#![cfg(feature = "worker")]

use std::cell::RefCell;

use worker::{Context, Env, Request, Response, Result, event};

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
    match env.service(&binding(&c)) {
        Ok(service) => service.fetch_request(req).await,
        Err(_) => Response::error(format!("no binding for {c}"), 503),
    }
}
