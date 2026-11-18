//! Web Push to the owner and the digest, in the control plane: devices and the latest push in
//! the `Instance` object, the pages a device subscribes from, and the hourly digest; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §3, §4.

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use worker::{Env, Fetch, Headers, Method, Request, RequestInit, Response, Result, State};

use super::approve as page;
use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use crate::access::Identity;
use crate::approve as pages;
use crate::push::{Devices, Latest, PAGE_SCRIPT, SERVICE_WORKER, Vapid};
use crate::tasks::{Digest, DigestInput, Tasks, digest};

/// The Worker secret holding the VAPID private key.
pub const VAPID_VAR: &str = "PUSH_VAPID_KEY";

const DEVICES: &str = "push-devices";
const LATEST: &str = "push-latest";
const CURSOR: &str = "digest-cursor";

/// Wakes every subscribed device with how many open feed items need the owner: nothing at 0 or
/// for the same count within a minute; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` §5. Logs rather than fails.
pub async fn notify(env: &Env, s: &Settings, _title: &str, _body: &str, _path: &str) {
    let count = open_count(env, s).await;
    if count == 0 {
        return;
    }
    let said = crate::core::app::push::said(count);
    if let Ok((_, v)) = to_instance(env, s, "/push/latest", &json!({})).await
        && let Ok(l) = serde_json::from_value::<Latest>(v)
        && l.body == said
        && now().saturating_sub(l.at) < 60
    {
        return;
    }
    if let Err(e) = send(env, s, &said).await {
        worker::console_error!("push: {e}");
    }
    if let Err(e) = super::apns::notify(env, s, count).await {
        worker::console_error!("apns: {e}");
    }
}

/// The open feed items that need the owner; 0 on any failure.
async fn open_count(env: &Env, s: &Settings) -> usize {
    let body = json!({ "after": 0, "owner": true, "needs": "owner", "open": true });
    to_instance(env, s, "/feed/read", &body)
        .await
        .ok()
        .and_then(|(_, v)| v["items"].as_array().map(Vec::len))
        .unwrap_or(0)
}

async fn send(env: &Env, s: &Settings, body: &str) -> Result<()> {
    let Ok(secret) = platform::secret(env, VAPID_VAR) else {
        return Ok(());
    };
    let vapid = Vapid::from_secret(&secret).map_err(worker::Error::RustError)?;
    let host = env
        .var(crate::wire::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    let latest = Latest {
        title: "pigeon".to_string(),
        body: body.to_string(),
        url: "/approvals".to_string(),
        at: now(),
    };
    to_instance(env, s, "/push/latest/put", &json!(latest)).await?;
    let (_, v) = to_instance(env, s, "/push/list", &json!({})).await?;
    let devices: Devices = serde_json::from_value(v).unwrap_or_default();
    for d in devices.0 {
        let auth = vapid
            .authorization(&d.endpoint, &format!("https://{host}"), now())
            .map_err(worker::Error::RustError)?;
        let headers = Headers::new();
        headers.set("authorization", &auth)?;
        headers.set("ttl", "86400")?;
        headers.set("urgency", "high")?;
        headers.set("content-length", "0")?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post).with_headers(headers);
        let status = match Fetch::Request(Request::new_with_init(&d.endpoint, &init)?)
            .send()
            .await
        {
            Ok(r) => r.status_code(),
            Err(e) => {
                worker::console_warn!("push to a device failed: {e}");
                continue;
            }
        };
        if status == 404 || status == 410 {
            to_instance(env, s, "/push/remove", &json!({ "endpoint": d.endpoint })).await?;
        } else if !(200..300).contains(&status) {
            worker::console_warn!("push answered {status}");
        }
    }
    Ok(())
}

/// The owner's digest now, from the instance's state and the events after the digest's cursor:
/// the digest, and the last event it read.
///
/// # Errors
///
/// When a binding fails.
pub async fn build(env: &Env, s: &Settings) -> Result<(Digest, u64)> {
    let (_, cursor) = to_instance(env, s, "/digest/cursor", &json!({})).await?;
    let after = cursor["after"].as_u64().unwrap_or_default();
    let waiting = page::live(env, s).await?;
    let (_, open_tasks) = to_instance(env, s, "/tasks/list", &json!({})).await?;
    let tasks = Tasks {
        last: 0,
        list: serde_json::from_value(open_tasks["tasks"].clone()).unwrap_or_default(),
    };
    let (_, all_mandates) = to_instance(env, s, "/mandates/list", &json!({})).await?;
    let mandates: Vec<crate::core::approval::mandates::Mandate> =
        serde_json::from_value(all_mandates["mandates"].clone()).unwrap_or_default();
    let events = since(env, s, after).await?;
    let last = events.last().map_or(after, |x| x.id);
    let failed: Vec<(String, String)> = events
        .iter()
        .filter(|x| x.kind == "deploy.failed")
        .map(|x| (String::new(), x.summary.clone()))
        .collect();
    let merged: Vec<(u32, String)> = events
        .iter()
        .filter(|x| x.kind == "change.approved")
        .filter_map(|x| {
            let by = x.summary.rsplit_once(" by ")?.1;
            ["mandate:", "policy:", "grant:", "window:"]
                .iter()
                .any(|p| by.starts_with(p))
                .then(|| (x.change.unwrap_or_default(), by.to_string()))
        })
        .collect();
    let behind = mirror_behind(env, s).await;
    let macos = macos_lines(env, &events).await;
    let containers = crate::tasks::containers(&events);
    let mut d = digest(
        &DigestInput {
            waiting: &waiting,
            tasks: Some(&tasks),
            mandates: &mandates,
            failed_deploys: &failed,
            merged_without_owner: &merged,
            mirror: behind.as_deref(),
            macos: &macos,
        },
        now(),
    );
    d.containers = containers;
    (d.sessions, d.stalled) = sessions(env, s).await?;
    d.unlisted = crate::tasks::unlisted(&d.sessions, &waiting);
    for n in &d.unlisted {
        let said = format!("!{n} needs the owner but had no approval listed; re-issued");
        let _ =
            super::events::append(env, s, "approval.unlisted", Some(*n), None, &said, true).await;
        if let Err(e) = super::to_change(env, &s.repo, *n, Method::Post, "/listed", None).await {
            worker::console_error!("listed for !{n}: {e}");
        }
    }
    let (_, w) = to_instance(env, s, "/windows/current", &json!({})).await?;
    let w: Option<crate::core::approval::windows::Window> =
        serde_json::from_value(w["window"].clone()).unwrap_or_default();
    if let Some((id, h)) = w.as_ref().and_then(|w| Some((w.id, w.hold.as_ref()?))) {
        let line = crate::tasks::held_line(&format!("Auto window {id}"), h, now());
        d.held.insert(0, line);
    }
    if let Some(line) = w.as_ref().and_then(window_line) {
        d.held.insert(0, line);
    }
    d.stops = crate::tasks::stops(&events);
    d.items = super::owner_items::lines(env, s).await?;
    d.queue = crate::tasks::queue_lines(&events);
    let open: std::collections::BTreeMap<u32, String> = d
        .sessions
        .iter()
        .flat_map(|l| l.changes.iter().map(|(n, stage, _)| (*n, stage.clone())))
        .collect();
    let briefs = super::briefs::all(env, s).await;
    d.briefs = crate::core::brief::report::lines(&briefs, &open, now());
    let reports: Vec<String> = events
        .iter()
        .filter(|x| x.kind == "session.archived")
        .map(|x| x.summary.clone())
        .collect();
    let (_, t) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(t).unwrap_or_default();
    d.archives = crate::tasks::archive_lines(&reports, &tickets.unarchived(now()));
    Ok((d, last))
}

/// The most `/events/since` pages the digest reads in one go.
const DIGEST_PAGES: usize = 50;

/// Every event after `after`, page by page; see
/// `.claude/decisions/2026-11-15-development-archive-cleanly-ended-sessions.md`, 2026-11-15.
async fn since(env: &Env, s: &Settings, mut after: u64) -> Result<Vec<crate::events::Event>> {
    let mut events = Vec::new();
    for _ in 0..DIGEST_PAGES {
        let body = json!({ "after": after, "owner": true });
        let (_, v) = to_instance(env, s, "/events/since", &body).await?;
        let page: Vec<crate::events::Event> =
            serde_json::from_value(v["events"].clone()).unwrap_or_default();
        let full = page.len() >= crate::events::PAGE;
        after = page.last().map_or(after, |e| e.id);
        events.extend(page);
        if !full || after >= v["last"].as_u64().unwrap_or_default() {
            break;
        }
    }
    Ok(events)
}

/// Each session with open changes, with whether it listens and each change's stage, and the
/// changes stalled on a session that isn't listening.
async fn sessions(
    env: &Env,
    s: &Settings,
) -> Result<(Vec<crate::tasks::SessionLine>, Vec<crate::tasks::Stall>)> {
    let rows = platform::d1_rows(env, &crate::control::open(&s.instance, &s.repo)).await?;
    let mut by: std::collections::BTreeMap<String, Vec<u32>> = std::collections::BTreeMap::new();
    for o in crate::control::open_rows(&rows) {
        if let Some(label) = o.session {
            by.entry(label).or_default().push(o.number);
        }
    }
    let q = super::stages::queue(env, s).await;
    let (_, y) = to_instance(env, s, "/yields/list", &json!({})).await?;
    let yields: std::collections::BTreeMap<u32, u32> =
        serde_json::from_value(y).unwrap_or_default();
    let (_, held) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(held).unwrap_or_default();
    let mut out = Vec::new();
    let mut stalls = Vec::new();
    for (label, numbers) in by {
        let model = tickets.class(&label);
        let (_, cursor) =
            to_instance(env, s, "/events/cursor", &json!({ "session": label })).await?;
        let listening = cursor["listening"].as_bool() == Some(true);
        let polled = cursor["polled"].as_u64();
        let mut changes = Vec::new();
        for n in numbers {
            let (_, view) = super::to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
            let Ok(c) =
                serde_json::from_value::<crate::core::change::Change>(view["change"].clone())
            else {
                continue;
            };
            let st = super::stages::stage(env, s, &c, &q).await;
            let needs = serde_json::to_value(st.needs)?;
            let needs = needs.as_str().unwrap_or_default().to_string();
            if !listening
                && let Some(quiet) = crate::tasks::stalled(polled, st.since, &needs, now())
            {
                stalls.push(crate::tasks::Stall {
                    number: n,
                    session: label.clone(),
                    stage: st.stage.clone(),
                    next: st.next.clone(),
                    quiet,
                    waiting: yields
                        .iter()
                        .filter(|(_, t)| **t == n)
                        .map(|(f, _)| *f)
                        .collect(),
                    model,
                });
            }
            changes.push((n, st.stage, needs));
        }
        out.push(crate::tasks::SessionLine {
            label,
            listening,
            changes,
            model,
        });
    }
    Ok((out, stalls))
}

/// The digest's macOS lines, from the events since the last digest and this month's minutes.
async fn macos_lines(env: &Env, events: &[crate::events::Event]) -> Vec<String> {
    let Ok(cfg) = crate::core::macos::config() else {
        return Vec::new();
    };
    let usage = super::macos::usage(env).await.unwrap_or_default();
    let pairs: Vec<(String, String)> = events
        .iter()
        .map(|x| (x.kind.clone(), x.summary.clone()))
        .collect();
    crate::core::macos::digest_lines(&pairs, &cfg, &usage)
}

/// Why the GitHub mirror needs the owner, per `MainRun`; none when it can't be asked.
async fn mirror_behind(env: &Env, s: &Settings) -> Option<String> {
    let req = platform::json_request("https://main/status", Method::Get, None, None).ok()?;
    let stub = super::main_stub(env, &s.repo).ok()?;
    let mut r = platform::call(env, &stub, req).await.ok()?;
    let v: Value = r.json().await.ok()?;
    v["mirror_behind"].as_str().map(str::to_string)
}

/// `digest.show`: the digest now; an approval's page link only for the owner.
///
/// # Errors
///
/// When a binding fails.
pub async fn show(env: &Env, s: &Settings, owner: bool) -> Result<Value> {
    let (mut d, _) = build(env, s).await?;
    if !owner {
        for a in &mut d.approvals {
            a.2 = format!("/approve/{}", a.0);
        }
    }
    Ok(json!({ "lines": d.lines(), "needs_owner": d.needs_owner(), "digest": d }))
}

/// The hourly digest: pushed when it needs the owner and differs from the last one pushed.
///
/// # Errors
///
/// When a binding fails.
pub async fn hourly(env: &Env) -> Result<()> {
    let s = Settings::from(env)?;
    // Silent sessions holding nothing lapse first; see the archive ADR's 2026-11-16 amendment.
    if let Err(e) = super::tickets::lapse(env, &s).await {
        worker::console_error!("lapse: {e}");
    }
    let (d, last) = build(env, &s).await?;
    let lines = d.owner_lines();
    let hash = crate::core::approval::grants::hex(&Sha256::digest(lines.join("\n").as_bytes()));
    let (_, cursor) = to_instance(env, &s, "/digest/cursor", &json!({})).await?;
    if d.needs_owner() && cursor["pushed"].as_str() != Some(hash.as_str()) {
        let more = lines.len().saturating_sub(1);
        let body = match more {
            0 => lines.first().cloned().unwrap_or_default(),
            n => format!("{} (and {n} more)", lines[0]),
        };
        notify(env, &s, "pigeon digest", &body, "/approvals").await;
        to_instance(
            env,
            &s,
            "/digest/cursor/put",
            &json!({ "after": last, "pushed": hash }),
        )
        .await?;
    } else {
        let pushed = cursor["pushed"].clone();
        to_instance(
            env,
            &s,
            "/digest/cursor/put",
            &json!({ "after": last, "pushed": pushed }),
        )
        .await?;
    }
    Ok(())
}

/// The `Instance` Durable Object's side: devices, the latest push and the digest's cursor.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    match path {
        "/push/add" | "/push/remove" | "/push/list" => {
            let mut d: Devices = storage.get(DEVICES).await?.unwrap_or_default();
            let endpoint = body["endpoint"].as_str().unwrap_or_default();
            match path {
                "/push/add" => {
                    if let Err(why) =
                        d.add(endpoint, body["by"].as_str().unwrap_or_default(), now())
                    {
                        return Response::error(why, 400);
                    }
                }
                "/push/remove" => d.remove(endpoint),
                _ => return Response::from_json(&d),
            }
            storage.put(DEVICES, &d).await?;
            Response::from_json(&json!({ "devices": d.0.len() }))
        }
        "/push/latest" => {
            let l: Latest = storage.get(LATEST).await?.unwrap_or_default();
            Response::from_json(&l)
        }
        "/push/latest/put" => {
            let l: Latest = serde_json::from_value(body.clone())?;
            storage.put(LATEST, &l).await?;
            Response::from_json(&json!({}))
        }
        "/digest/cursor" => {
            let c: Value = storage.get(CURSOR).await?.unwrap_or_else(|| json!({}));
            Response::from_json(&c)
        }
        "/digest/cursor/put" => {
            storage.put(CURSOR, body).await?;
            Response::from_json(&json!({}))
        }
        _ => Response::error("not found", 404),
    }
}

fn script(body: &str) -> Result<Response> {
    let mut r = Response::ok(body)?;
    r.headers_mut()
        .set("content-type", "text/javascript; charset=utf-8")?;
    r.headers_mut().set("cache-control", "no-store")?;
    Ok(r)
}

/// Serves the pages a device subscribes from, to the owner only.
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
        return page::refused(403, "only a person signed in to Access subscribes a device");
    }
    match (req.method(), req.path().as_str()) {
        (Method::Get, "/sw.js") => script(SERVICE_WORKER),
        (Method::Get, "/approvals/manifest.json") => {
            let mut r = Response::ok(pages::MANIFEST)?;
            r.headers_mut()
                .set("content-type", "application/manifest+json")?;
            Ok(r)
        }
        (Method::Get, "/approvals/push.js") => script(PAGE_SCRIPT),
        (Method::Get, "/approvals/vapid") => match platform::secret(env, VAPID_VAR)
            .ok()
            .and_then(|k| Vapid::from_secret(&k).ok())
        {
            Some(v) => Response::ok(v.public_key()),
            None => Response::error("push isn't set up: no PUSH_VAPID_KEY", 503),
        },
        (Method::Get, "/approvals/latest.json") => {
            let (_, v) = to_instance(env, s, "/push/latest", &json!({})).await?;
            Response::from_json(&v)
        }
        (Method::Post, "/approvals/push") => {
            let url = req.url()?;
            let host = url.host_str().unwrap_or_default().to_string();
            let origin = req.headers().get("origin")?;
            let site = req.headers().get("sec-fetch-site")?;
            if !pages::same_origin(origin.as_deref(), site.as_deref(), &host) {
                return Response::error("subscribed only from its own page", 403);
            }
            let body: Value = req.json().await?;
            let by = super::account::approver(env, s).await?;
            let add = json!({ "endpoint": body["endpoint"], "by": by });
            let (status, v) = to_instance(env, s, "/push/add", &add).await?;
            Ok(Response::from_json(&v)?.with_status(status))
        }
        (method, path) => Response::error(format!("{method:?} {path} isn't served here"), 404),
    }
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::any(crate::routes::Path::Exact("/sw.js"), serve_route),
    super::Route::any(
        crate::routes::Path::Exact("/approvals/push.js"),
        serve_route,
    ),
    super::Route::any(crate::routes::Path::Exact("/approvals/vapid"), serve_route),
    super::Route::any(crate::routes::Path::Exact("/approvals/push"), serve_route),
    super::Route::any(
        crate::routes::Path::Exact("/approvals/latest.json"),
        serve_route,
    ),
    super::Route::any(
        crate::routes::Path::Exact("/approvals/manifest.json"),
        serve_route,
    ),
];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
}

/// The digest's line for an Auto window the rate breaker holds or that is paused, which a
/// deploy hold's line doesn't cover; see the standing Auto ADR's 2026-11-15 amendment on the
/// rate breaker.
fn window_line(w: &crate::core::approval::windows::Window) -> Option<String> {
    use crate::core::approval::mandates::State;
    let why = w.why.clone().unwrap_or_default();
    match w.state {
        State::Held if w.quiet.is_some() => Some(format!("held: Auto window {}: {why}", w.id)),
        State::Paused => Some(format!(
            "paused: Auto window {}: {why}; resume it in the app",
            w.id
        )),
        _ => None,
    }
}
