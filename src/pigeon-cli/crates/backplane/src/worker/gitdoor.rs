//! The git front door: smart HTTP proxied to Artifacts with a token minted per request, and a
//! push judged first; see `.claude/decisions/2026-11-06-security-git-front-door-spike.md` §2.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use worker::js_sys::Uint8Array;
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, Response, Result};

use super::platform::{self, Settings, now};
use super::to_change;
use crate::access::Identity;
use crate::changes::Change;
use crate::control;
use crate::gitdoor::{self, Service};

/// How long a front-door token lives, in seconds; one request's worth.
const TTL: u64 = 120;

/// git's headers a request keeps on its way to Artifacts.
const FORWARDED: [&str; 4] = ["content-type", "accept", "git-protocol", "content-encoding"];

/// Serves `/git/…` for `who`.
///
/// # Errors
///
/// When a binding or the Artifacts API fails.
pub async fn serve(mut req: Request, env: &Env, s: &Settings, who: &Identity) -> Result<Response> {
    let path = req.path();
    let Some((ns, repo, service)) = gitdoor::path(&path) else {
        return Response::error("not found", 404);
    };
    if ns != s.namespace || repo != s.repo {
        return Response::error("not found", 404);
    }
    let method = req.method();
    let query = req.url()?.query().map(str::to_string);
    let write = match (&method, service) {
        (Method::Get, Service::InfoRefs) => query
            .as_deref()
            .is_some_and(|q| q.contains("service=git-receive-pack")),
        (Method::Post, Service::UploadPack) => false,
        (Method::Post, Service::ReceivePack) => true,
        _ => return Response::error("method not allowed", 405),
    };
    let body = if method == Method::Post {
        Some(req.bytes().await?)
    } else {
        None
    };
    if service == Service::ReceivePack {
        let bytes = body.as_deref().unwrap_or_default();
        let push = match gitdoor::commands(bytes) {
            Ok(p) => p,
            Err(why) => return Response::error(why, 400),
        };
        let refusals = judge(env, s, &push).await?;
        let numbers: Vec<String> = push
            .updates
            .iter()
            .filter_map(|u| u.change().map(|(n, _)| n.to_string()))
            .collect();
        let result = if refusals.is_empty() { "ok" } else { "refused" };
        let audit = control::audit(
            now(),
            &who.subject,
            "git.push",
            &s.instance,
            Some(numbers.join(",").as_str()).filter(|t| !t.is_empty()),
            result,
        );
        if let Err(e) = platform::d1(env, &[control::instance_row(&s.instance, now()), audit]).await
        {
            worker::console_error!("audit: {e}");
        }
        if !refusals.is_empty() {
            let headers = Headers::new();
            headers.set("content-type", "application/x-git-receive-pack-result")?;
            headers.set("cache-control", "no-cache")?;
            return Ok(
                Response::from_bytes(gitdoor::refusal(&push, &refusals))?.with_headers(headers)
            );
        }
    }
    forward(env, &req, &path, query.as_deref(), &method, body, write).await
}

/// Every refused ref of `push`, with why.
async fn judge(env: &Env, s: &Settings, push: &gitdoor::Push) -> Result<Vec<(String, String)>> {
    let mut changes: BTreeMap<u32, Option<Change>> = BTreeMap::new();
    let mut refusals = Vec::new();
    for u in &push.updates {
        let change = match u.change() {
            Some((n, _)) => {
                if let Entry::Vacant(e) = changes.entry(n) {
                    let (status, v) =
                        to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
                    let c = (status == 200)
                        .then(|| serde_json::from_value(v["change"].clone()).ok())
                        .flatten();
                    e.insert(c);
                }
                changes.get(&n).cloned().flatten()
            }
            None => None,
        };
        if let Err(why) = gitdoor::judge(u, change.as_ref()) {
            refusals.push((u.name.clone(), why));
        }
    }
    Ok(refusals)
}

/// Sends the request on to the repository's remote with a minted token; Artifacts' answer
/// comes back as it is, or as `502` when it refuses, so git never asks for a password.
async fn forward(
    env: &Env,
    req: &Request,
    path: &str,
    query: Option<&str>,
    method: &Method,
    body: Option<Vec<u8>>,
    write: bool,
) -> Result<Response> {
    let (remote, token) = platform::credentials(env, write, TTL).await?;
    let endpoint = path.split_once(".git/").map_or("", |(_, e)| e);
    let mut url = format!("{}/{endpoint}", remote.trim_end_matches('/'));
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    let headers = Headers::new();
    headers.set("authorization", &format!("Bearer {token}"))?;
    for h in FORWARDED {
        if let Some(v) = req.headers().get(h)? {
            headers.set(h, &v)?;
        }
    }
    let mut init = RequestInit::new();
    init.with_method(method.clone()).with_headers(headers);
    if let Some(b) = body {
        init.with_body(Some(Uint8Array::from(b.as_slice()).into()));
    }
    let r = Fetch::Request(Request::new_with_init(&url, &init)?)
        .send()
        .await?;
    let status = r.status_code();
    if (200..300).contains(&status) {
        return Ok(r);
    }
    Response::error(format!("Artifacts answered {status}"), 502)
}
