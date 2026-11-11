//! The merge queue in the control plane: `change.merge` asks the `Instance` Durable Object for a
//! turn, and each merge that ends starts the next and marks the open changes behind; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::to_change;
use crate::changes::{Change, Event};
use crate::control;
use crate::merges::{Entry, MergeQueue, Queued};
use crate::reconcile::{PROPOSER, Proposal};
use crate::sessions;

/// The storage key of the merge queue.
const MERGES: &str = "merges";

/// The most queued merges tried in a row when each refuses to start.
const TRIES: usize = 10;

/// `change.merge` on change `number` for `by`: started now, or queued, with its place.
///
/// # Errors
///
/// When a binding fails.
pub async fn ask(env: &Env, s: &Settings, number: u32, by: &str) -> Result<(u16, Value)> {
    let (status, v) = to_instance(
        env,
        s,
        "/merges/enqueue",
        &json!({ "number": number, "by": by }),
    )
    .await?;
    if status != 200 {
        return Ok((409, v));
    }
    if let Ok(e) = serde_json::from_value::<Entry>(v["resumed"].clone()) {
        start(env, s, Some(e)).await?;
    }
    if let Ok(Queued::Behind(place)) = serde_json::from_value::<Queued>(v["queued"].clone()) {
        let message = format!("change {number} merges after {place} other merge(s)");
        return Ok((202, json!({ "queued": place, "message": message })));
    }
    let (status, v) = merge(env, s, number, by).await?;
    if status != 200 {
        let (_, next) = to_instance(env, s, "/merges/done", &json!({ "number": number })).await?;
        start(env, s, serde_json::from_value(next["next"].clone()).ok()).await?;
        return Ok((status, v));
    }
    Ok((status, json!({ "change": v })))
}

/// Change `c`'s merge ended, as `merged`, the commit `main` now holds, or failed: marks every
/// other open change behind `main`, which restacks the checked and approved ones, plans a
/// review for each open change in the area `c`'s ADR decides, then starts the next merge.
///
/// # Errors
///
/// When a binding fails.
pub async fn ended(env: &Env, c: &Change, merged: Option<&str>) -> Result<()> {
    let s = Settings::from(env)?;
    if merged.is_some() {
        super::tasks::close_for(env, &s, c.number).await?;
    }
    if let Some(main) = merged {
        let rows = platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?;
        let body = serde_json::to_value(Event::Behind {
            main: main.to_string(),
        })?;
        for open in control::open_rows(&rows) {
            if open.number == c.number {
                continue;
            }
            let _ = to_change(
                env,
                &s.repo,
                open.number,
                Method::Post,
                "/event",
                Some(&body),
            )
            .await;
            let globs: Vec<String> = c
                .affects
                .iter()
                .filter(|a| {
                    open.scope
                        .iter()
                        .chain(&open.affects)
                        .any(|o| sessions::meet(a, o))
                })
                .cloned()
                .collect();
            if globs.is_empty() {
                continue;
            }
            if let Some(p) =
                crate::reconcile::review(open.number, &open.plan, c.number, &c.plan, &globs)
                && let Err(e) = reconcile(env, open.number, open.session.clone(), p).await
            {
                worker::console_error!("review of change {}: {e}", open.number);
            }
        }
    }
    let (_, v) = to_instance(env, &s, "/merges/done", &json!({ "number": c.number })).await?;
    start(env, &s, serde_json::from_value(v["next"].clone()).ok()).await
}

/// Proposes reconciliation `p` of change `parent`, for its `session`: the new change's
/// number, if proposed.
///
/// # Errors
///
/// When a binding fails.
pub async fn reconcile(
    env: &Env,
    parent: u32,
    session: Option<String>,
    p: Proposal,
) -> Result<Option<u32>> {
    let s = Settings::from(env)?;
    let mut change = Change::propose(0, &p.slug, &p.title, &p.plan, p.scope, PROPOSER);
    change.session = session;
    change.parent = Some(parent);
    let session = change.session.clone();
    let (number, r) = super::propose(env, &s, "change.propose", change).await?;
    if !r.ok {
        worker::console_warn!("reconcile {parent}: {}", r.violations.join("; "));
    }
    let number: Option<u32> = number.and_then(|n| n.parse().ok());
    if let Some(n) = number {
        let kind = p.slug.split('-').next().unwrap_or("reconcile");
        super::tasks::add(env, &s, kind, n, Some(parent), session, &p.title).await?;
    }
    Ok(number)
}

async fn merge(env: &Env, s: &Settings, number: u32, by: &str) -> Result<(u16, Value)> {
    let body = serde_json::to_value(Event::Merge { by: by.to_string() })?;
    to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await
}

/// Starts `next`, and past each one that refuses, the one after it.
async fn start(env: &Env, s: &Settings, mut next: Option<Entry>) -> Result<()> {
    for _ in 0..TRIES {
        let Some(e) = next.take() else {
            return Ok(());
        };
        let (status, v) = merge(env, s, e.number, &e.by).await?;
        if status == 200 {
            return Ok(());
        }
        worker::console_warn!("merge queue: change {} didn't start: {v}", e.number);
        let answer: String = v.to_string().chars().take(300).collect();
        let why = format!("!{} was passed over in the merge queue: {answer}", e.number);
        let added =
            super::events::append(env, s, "merge.skipped", Some(e.number), None, &why, false);
        if let Err(err) = added.await {
            worker::console_error!("merge.skipped: {err}");
        }
        let (_, v) = to_instance(env, s, "/merges/done", &json!({ "number": e.number })).await?;
        next = serde_json::from_value(v["next"].clone()).ok();
    }
    Ok(())
}

/// The `Instance` Durable Object's side: `/merges/enqueue`, `done` and `list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut q: MergeQueue = storage.get(MERGES).await?.unwrap_or_default();
    let number = body["number"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_default();
    let answer = match path {
        "/merges/enqueue" => {
            let resumed = q.unstick(now());
            match q.enqueue(number, body["by"].as_str().unwrap_or_default(), now()) {
                Ok(queued) => json!({ "queued": queued, "resumed": resumed }),
                Err(why) => {
                    storage.put(MERGES, &q).await?;
                    return Response::error(why, 409);
                }
            }
        }
        "/merges/done" => json!({ "next": q.done(number, now()) }),
        "/merges/list" => return Response::from_json(&q),
        _ => return Response::error("not found", 404),
    };
    storage.put(MERGES, &q).await?;
    Response::from_json(&answer)
}
