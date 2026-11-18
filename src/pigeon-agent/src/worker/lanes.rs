//! The owner's fast lane in the control plane: the `lane.*` operations, the `Instance` Durable
//! Object's store of lanes, and `/lane/<id>`, the page that opens one with the owner's passkey;
//! see `.claude/decisions/2026-11-17-security-owner-fast-lane-window.md`.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::approval::lanes::{self, Lane, Lanes, Line};
use crate::core::change::{Change, Event};

/// The storage key of every lane.
const LANES: &str = "lanes";

/// How many ended lanes `lane.status` shows.
const SHOWN: usize = 10;

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
    .filter(|s| !s.trim().is_empty())
}

/// The line `args` names: exactly one of `session`, `changes` or `paths`.
fn line(args: &Map<String, Value>) -> std::result::Result<Line, String> {
    let session = text(args, "session");
    let changes = text(args, "changes");
    let paths = text(args, "paths");
    match (session, changes, paths) {
        (Some(s), None, None) => Ok(Line::Session(s)),
        (None, Some(c), None) => c
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|n| !n.is_empty())
            .map(|n| {
                n.trim_start_matches('!')
                    .parse::<u32>()
                    .map_err(|_| format!("`{n}` is not a change number"))
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(Line::Changes),
        (None, None, Some(p)) => Ok(Line::Paths(p.lines().map(str::to_string).collect())),
        _ => Err("name exactly one of `session`, `changes` or `paths`".into()),
    }
}

/// Appends a lane event to the instance's log, under `subject`, logging a failure.
async fn log(env: &Env, s: &Settings, kind: &str, subject: &str, said: &str, owner: bool) {
    let body = json!({
        "kind": kind, "change": null, "session": null, "summary": said, "owner": owner,
        "subject": subject,
    });
    if let Err(e) = to_instance(env, s, "/events/add", &body).await {
        worker::console_error!("{kind}: {e}");
    }
}

/// Runs `operation` with `args` for `who`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    let caller = super::tickets::holder(env, s, text(args, "ticket").as_deref())
        .await?
        .ok();
    match operation {
        "lane.request" => {
            let line = match line(args) {
                Ok(l) => l,
                Err(why) => return Ok((400, json!(why))),
            };
            let token = platform::random_hex(32)?;
            let body = json!({
                "line": line,
                "minutes": text(args, "minutes").and_then(|m| m.parse::<u32>().ok()),
                "reason": text(args, "reason").unwrap_or_default(),
                "asked_by": caller.clone().unwrap_or_else(|| who.subject.clone()),
                "extends": text(args, "extends").and_then(|i| i.parse::<u32>().ok()),
                "token": token,
            });
            let (status, v) = to_instance(env, s, "/lanes/request", &body).await?;
            if status != 200 {
                return Ok((status, v));
            }
            let l: Lane = serde_json::from_value(v)?;
            let url = offer(env, s, &l, &token).await?;
            let said = format!("fast lane {} asked: {}", l.id, l.reason);
            log(
                env,
                s,
                "lane.requested",
                &format!("lane-request:{}", l.id),
                &said,
                true,
            )
            .await;
            Ok((
                202,
                json!({ "lane": l, "pending": { "operation": "lane.grant", "link": url } }),
            ))
        }
        "lane.status" => to_instance(env, s, "/lanes/list", &json!({})).await,
        "lane.end" => {
            let Some(id) = text(args, "id").and_then(|i| i.parse::<u32>().ok()) else {
                return Ok((400, json!("`id` must be a lane number")));
            };
            let (_, v) = to_instance(env, s, "/lanes/list", &json!({})).await?;
            let named = serde_json::from_value::<Vec<Lane>>(v["lanes"].clone())
                .unwrap_or_default()
                .into_iter()
                .find(|l| l.id == id)
                .is_some_and(|l| {
                    matches!((&l.line, caller.as_deref()), (Line::Session(a), Some(b)) if a == b)
                });
            if !who.human && !named {
                return Ok((
                    403,
                    json!("only the owner, or a session the lane names, ends a fast lane"),
                ));
            }
            let who = caller.unwrap_or_else(|| who.subject.clone());
            let why = text(args, "why").map_or_else(
                || format!("ended by {who}"),
                |w| format!("ended by {who}: {w}"),
            );
            end(env, s, id, &why).await
        }
        other => Ok((404, json!(format!("`{other}` is not a lane operation")))),
    }
}

/// Ends lane `id` saying `why`, and logs `lane.ended`: its status and answer.
async fn end(env: &Env, s: &Settings, id: u32, why: &str) -> Result<(u16, Value)> {
    let (status, v) = to_instance(env, s, "/lanes/end", &json!({ "id": id, "why": why })).await?;
    if status == 200 {
        log(
            env,
            s,
            "lane.ended",
            &format!("lane:{id}"),
            &format!("fast lane {id} {why}"),
            false,
        )
        .await;
        if let Ok(l) = serde_json::from_value::<Lane>(v.clone()) {
            unwaive(env, s, &l).await;
        }
    }
    Ok((status, v))
}

/// Runs again the jobs ended lane `l` waived, on each change it waived them on; see the ADR's §5.
async fn unwaive(env: &Env, s: &Settings, l: &Lane) {
    let mut changes: Vec<u32> = l.waived.iter().map(|w| w.change).collect();
    changes.sort_unstable();
    changes.dedup();
    let Ok(body) = serde_json::to_value(Event::Unwaived { lane: l.id }) else {
        return;
    };
    for n in changes.into_iter().filter(|n| !l.merged.contains(n)) {
        let sent = super::to_change(env, &s.repo, n, Method::Post, "/event", Some(&body)).await;
        if let Err(e) = sent {
            worker::console_error!("lane {} unwaive !{n}: {e}", l.id);
        }
    }
}

/// Ends the open lane, if any, saying `why`: after a failed deploy; see the ADR's §6.
///
/// # Errors
///
/// When a binding fails.
pub async fn end_open(env: &Env, s: &Settings, why: &str) -> Result<()> {
    let (_, v) = to_instance(env, s, "/lanes/current", &json!({})).await?;
    if let Some(id) = v["lane"]["id"].as_u64().and_then(|i| u32::try_from(i).ok()) {
        end(env, s, id, why).await?;
    }
    Ok(())
}

/// Whether the open lane covers `change`'s head: the lane's number, or why not.
///
/// # Errors
///
/// When a binding fails.
pub async fn cover(
    env: &Env,
    s: &Settings,
    change: &Change,
) -> Result<std::result::Result<u32, String>> {
    let (_, v) = to_instance(env, s, "/lanes/cover", &json!({ "change": change })).await?;
    Ok(
        match v["lane"].as_u64().and_then(|i| u32::try_from(i).ok()) {
            Some(id) if v["ok"].as_bool() == Some(true) => Ok(id),
            _ => Err(v["why"].as_str().unwrap_or("no lane covers it").to_string()),
        },
    )
}

/// Records on the open lane that it waived `job` on `change`'s head.
///
/// # Errors
///
/// When a binding fails.
pub async fn record_waiver(env: &Env, s: &Settings, change: &Change, job: &str) -> Result<()> {
    let body = json!({ "change": change, "job": job });
    let (status, v) = to_instance(env, s, "/lanes/waive", &body).await?;
    if status != 200 {
        worker::console_warn!("lane waiver of {job} on !{}: {v}", change.number);
    }
    Ok(())
}

/// Lists request `l`'s page in the inbox, under `token`, and answers its link.
async fn offer(env: &Env, s: &Settings, l: &Lane, token: &str) -> Result<String> {
    let path = format!("/lane/{}?grant={token}", l.id);
    let waiting = Waiting {
        number: l.id,
        operation: "lane.grant".into(),
        title: l.reason.clone(),
        path: path.clone(),
        expires: now() + lanes::OFFER_SECONDS,
    };
    elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await?;
    let host = env
        .var(crate::wire::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok(super::app_link(&host, &path))
}

/// The lanes `lane.status` shows: the open one and the last [`SHOWN`], newest first.
fn shown(all: &Lanes) -> Vec<Lane> {
    all.list
        .iter()
        .rev()
        .filter(|l| l.state != lanes::State::Offered)
        .take(SHOWN)
        .cloned()
        .collect()
}

/// The `Instance` Durable Object's side: `/lanes/request`, `list`, `current`, `peek`, `grant`,
/// `end`, `settle`, `cover`, `waive` and `merged`. Lanes past their end are ended on each call.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let all: Lanes = storage.get(LANES).await?.unwrap_or_default();
    let t = now();
    let (all, expired) = all.settle(t);
    let n = |k: &str| body[k].as_u64().and_then(|v| u32::try_from(v).ok());
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let change = || serde_json::from_value::<Change>(body["change"].clone());
    let (all, answer): (Lanes, std::result::Result<Value, String>) = match path {
        "/lanes/request" => match serde_json::from_value::<Line>(body["line"].clone()) {
            Ok(line) => {
                let (all, asked) = all.request(
                    line,
                    n("minutes"),
                    &s("reason"),
                    &s("asked_by"),
                    n("extends"),
                    &s("token"),
                    t,
                );
                (all, asked.map(|l| json!(l)))
            }
            Err(e) => (all, Err(format!("unreadable line: {e}"))),
        },
        "/lanes/list" => {
            let open = all.current(t).cloned();
            (
                all.clone(),
                Ok(json!({ "open": open, "lanes": shown(&all) })),
            )
        }
        "/lanes/current" => {
            let open = all.current(t).cloned();
            (all, Ok(json!({ "lane": open })))
        }
        "/lanes/settle" => (all, Ok(json!({ "ended": expired }))),
        "/lanes/peek" => {
            return match all.peek(&s("token"), t) {
                Ok(l) => Response::from_json(l),
                Err(why) => Response::error(why, 403),
            };
        }
        "/lanes/grant" => {
            let (all, granted) = all.grant(&s("token"), &s("by"), t);
            (all, granted.map(|l| json!(l)))
        }
        "/lanes/end" => match n("id") {
            Some(id) => {
                let (all, ended) = all.end(id, &s("why"));
                (all, ended.map(|l| json!(l)))
            }
            None => (all, Err("`id` must be a lane number".into())),
        },
        "/lanes/cover" => match change() {
            Ok(c) => {
                let open = all.current(t);
                let answer = match open.map(|l| (l.id, lanes::covers(l, &c, t))) {
                    Some((id, Ok(()))) => json!({ "lane": id, "ok": true }),
                    Some((id, Err(why))) => json!({ "lane": id, "ok": false, "why": why }),
                    None => json!({ "lane": null, "ok": false, "why": "no lane is open" }),
                };
                (all, Ok(answer))
            }
            Err(e) => return Response::error(format!("unreadable change: {e}"), 400),
        },
        "/lanes/waive" => match change() {
            Ok(c) => match all.waive(&c, &s("job"), t) {
                Ok((all, id)) => (all, Ok(json!({ "lane": id }))),
                Err(why) => (all, Err(why)),
            },
            Err(e) => return Response::error(format!("unreadable change: {e}"), 400),
        },
        "/lanes/merged" => match (n("id"), n("change")) {
            (Some(id), Some(c)) => (all.merged(id, c), Ok(json!({ "lane": id }))),
            _ => (all, Err("`id` and `change` must be numbers".into())),
        },
        _ => return Response::error("not found", 404),
    };
    storage.put(LANES, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}

/// The lane number in `/lane/<id>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/lane/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Serves `/lane/<id>` for `who`: the page, and the POST that opens it with the owner's passkey.
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
        return page::refused(403, "only a person signed in to Access opens a fast lane");
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
                to_instance(env, s, "/lanes/peek", &json!({ "token": token })).await?;
            match serde_json::from_value::<Lane>(v.clone()) {
                Ok(l) if status == 200 && l.id == id => {
                    page::html_with(200, &approve::lane_page(&l, &token), approve::PROOF_CSP)
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
            let purpose = crate::core::approval::passkeys::grant_purpose(&token);
            let shown = format!("/lane/{id}?grant={token}");
            let proved = super::passkeys::on_page(env, s, &form, &purpose, &host, &shown).await?;
            if let Err(why) = proved {
                return page::refused(403, &why);
            }
            let (status, v) = to_instance(
                env,
                s,
                "/lanes/grant",
                &json!({ "token": token, "by": super::account::approver(env, s).await? }),
            )
            .await?;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": shown })).await;
            answered(env, s, who, id, status, &v).await
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// The page after request `id`'s grant answered `status` and `v`, with its events logged.
async fn answered(
    env: &Env,
    s: &Settings,
    who: &Identity,
    id: u32,
    status: u16,
    v: &Value,
) -> Result<Response> {
    let request = format!("lane-request:{id}");
    let Ok(l) = serde_json::from_value::<Lane>(v.clone())
        .map_err(|_| ())
        .and_then(|l| if status == 200 { Ok(l) } else { Err(()) })
    else {
        let why = v.as_str().unwrap_or("the lane was not opened").to_string();
        let said = format!("fast lane request {id}: {why}");
        log(env, s, "lane.refused", &request, &said, true).await;
        return page::refused(409, &why);
    };
    let said = format!("fast lane request {id} granted");
    log(env, s, "lane.granted", &request, &said, true).await;
    let (kind, said) = if l.id == id {
        (
            "lane.open",
            format!("fast lane {} open: {}", l.id, l.reason),
        )
    } else {
        (
            "lane.extended",
            format!("fast lane {} widened by request {id}", l.id),
        )
    };
    log(env, s, kind, &format!("lane:{}", l.id), &said, false).await;
    let until = l.until.unwrap_or_default();
    page::html(
        200,
        &approve::outcome(
            &format!("Fast lane {} open", l.id),
            &format!(
                "{}\nOpen until {until} (Unix seconds). Recorded as {}.",
                l.reason, who.subject
            ),
        ),
    )
}

/// This page's routes, reached only through `page_post` (`proof.submit`), not served; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendments.
pub(super) const PAGES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix("/lane/"),
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
