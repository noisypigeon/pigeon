//! The merge queue in the control plane: `change.merge` asks the `Instance` Durable Object for a
//! turn, and each merge that ends starts the next and marks the open changes behind; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::to_change;
use crate::control;
use crate::core::change::queue::{self as merges, Back, Entry, MergeQueue, Queued};
use crate::core::change::{Change, Event};
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
    hand_back_idle(env, s).await?;
    asked(env, s, number, by).await?;
    let own = record(env, s, number).await?;
    let docs = own.as_ref().is_some_and(merges::docs_only);
    let back = own.as_ref().map_or_else(Back::default, Back::of);
    let (status, v) = to_instance(
        env,
        s,
        "/merges/enqueue",
        &json!({ "number": number, "by": by, "docs": docs, "back": back }),
    )
    .await?;
    // A turn the queue moved past starts, whatever it answers this ask.
    if let Ok(e) = serde_json::from_value::<Entry>(v["resumed"].clone()) {
        start(env, s, Some(e)).await?;
    }
    if status != 200 {
        return Ok((409, v["error"].clone()));
    }
    if let Some(why) = v["returned"].as_str() {
        let why = format!("!{number} takes its place back in the merge queue: {why}");
        let added =
            super::events::append(env, s, "merge.returned", Some(number), None, &why, false);
        if let Err(e) = added.await {
            worker::console_error!("merge.returned: {e}");
        }
    }
    if let Ok(Queued::Behind(place)) = serde_json::from_value::<Queued>(v["queued"].clone()) {
        let message = format!("change {number} merges after {place} other merge(s)");
        return Ok((202, json!({ "queued": place, "message": message })));
    }
    if let Ok(Queued::Yielded { place, to }) = serde_json::from_value(v["queued"].clone()) {
        let message =
            format!("change {number} keeps place {place}; it merges once its yield to !{to} ends");
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

/// Starts the next merge whose yield ended, if none runs; see the yield ADR's 2026-11-13
/// amendment on the queue keeping a yielded change's place.
///
/// # Errors
///
/// When a binding fails.
pub async fn resume(env: &Env, s: &Settings) -> Result<()> {
    let (_, v) = to_instance(env, s, "/merges/resume", &json!({})).await?;
    start(env, s, serde_json::from_value(v["next"].clone()).ok()).await
}

/// The release bot asks for the merge turn, its target having waited `waited` seconds: whether
/// it holds it; see the merge-path ADR §1.
///
/// # Errors
///
/// When a binding fails.
pub async fn release_turn(env: &Env, s: &Settings, waited: u64) -> Result<bool> {
    let (_, v) = to_instance(env, s, "/merges/release-turn", &json!({ "waited": waited })).await?;
    Ok(v["granted"].as_bool().unwrap_or(false))
}

/// The release bot's turn ended: the next merge starts, if any.
///
/// # Errors
///
/// When a binding fails.
pub async fn release_turn_done(env: &Env, s: &Settings) -> Result<()> {
    let (_, v) = to_instance(env, s, "/merges/release-turn-done", &json!({})).await?;
    start(env, s, serde_json::from_value(v["next"].clone()).ok()).await
}

/// `change` lost its turn only because `main` moved: the queue keeps it and starts nothing,
/// `merge.kept` is recorded, its merge is asked again as its session would, and its reservation
/// is released, so the queue moves on only when that didn't take the turn back; see the
/// concurrent-sessions ADR's 2026-11-15 amendment.
///
/// # Errors
///
/// When a binding fails.
pub async fn kept(env: &Env, change: &Change) -> Result<()> {
    let s = Settings::from(env)?;
    let n = change.number;
    let (_, answer) = to_instance(env, &s, "/merges/keep", &json!({ "number": n })).await?;
    let misses = answer["misses"].as_u64().unwrap_or(1);
    let why = format!(
        "!{n} keeps its merge turn: main moved under it ({misses} in a row); its merge is asked again"
    );
    let added = super::events::append(
        env,
        &s,
        "merge.kept",
        Some(n),
        change.session.clone(),
        &why,
        false,
    );
    if let Err(e) = added.await {
        worker::console_error!("merge.kept: {e}");
    }
    ask_again(env, &s, n, "kept").await;
    let (_, released) = to_instance(env, &s, "/merges/release", &json!({ "number": n })).await?;
    start(
        env,
        &s,
        serde_json::from_value(released["next"].clone()).ok(),
    )
    .await
}

/// `after`, saved over `before`, had its approval carried with no merge on its way, after its
/// merge was asked: `merge.reasked` is recorded and its merge asked again, as its session would;
/// whether it was. See the merge-path ADR §4.
pub async fn reask(
    env: &Env,
    s: &Settings,
    before: Option<&Change>,
    after: &Change,
    q: &MergeQueue,
) -> bool {
    let Some(by) = merges::reask(before, after, q) else {
        return false;
    };
    let n = after.number;
    let why = format!(
        "!{n}'s approval carried to its new head; its merge, asked by {by}, is asked again"
    );
    let session = after.session.clone();
    let added = super::events::append(env, s, "merge.reasked", Some(n), session, &why, false);
    if let Err(e) = added.await {
        worker::console_error!("merge.reasked: {e}");
    }
    ask_again(env, s, n, "reasked").await;
    true
}

/// Asks `change.merge` on change `n` as its session would; `what` names the case in the log.
async fn ask_again(env: &Env, s: &Settings, n: u32, what: &str) {
    let mut args = serde_json::Map::new();
    args.insert("number".into(), json!(n));
    let request = crate::wire::Request {
        operation: "change.merge".into(),
        args,
    };
    match Box::pin(super::ask(env, s, n, &request)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => worker::console_warn!("merge of {what} !{n}: {e}"),
        Err(e) => worker::console_error!("merge of {what} !{n}: {e}"),
    }
}

/// Records on change `number` that `by` asked for its merge; see the merge-path ADR §4.
async fn asked(env: &Env, s: &Settings, number: u32, by: &str) -> Result<()> {
    let body = serde_json::to_value(Event::MergeAsked { by: by.to_string() })?;
    let (status, v) = to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await?;
    if status != 200 {
        worker::console_warn!("merge asked on !{number}: {v}");
    }
    Ok(())
}

/// `change.front` on change `number`, by the owner `by`: moved to the front of the queue's
/// waiting line, recorded as `merge.fronted`; see the concurrent-sessions ADR's 2026-11-15
/// amendment on the owner moving a change to the front.
///
/// # Errors
///
/// When a binding fails.
pub async fn front(env: &Env, s: &Settings, number: u32, by: &str) -> Result<(u16, Value)> {
    let (status, answer) =
        to_instance(env, s, "/merges/front", &json!({ "number": number })).await?;
    if status != 200 {
        return Ok((409, answer["error"].clone()));
    }
    let (place, of) = (&answer["left"][0], &answer["left"][1]);
    let why =
        format!("!{number} was moved to the front of the merge queue by {by}, from {place}/{of}");
    let added = super::events::append(env, s, "merge.fronted", Some(number), None, &why, false);
    if let Err(e) = added.await {
        worker::console_error!("merge.fronted: {e}");
    }
    resume(env, s).await?;
    Ok((
        200,
        json!({ "fronted": number, "from": answer["left"], "message": why }),
    ))
}

/// Hands back the turn of a holder with no merge on its way, recording `merge.released`; see the
/// 2026-11-11 amendment on a turn held by no merge.
async fn hand_back_idle(env: &Env, s: &Settings) -> Result<()> {
    let (_, queue) = to_instance(env, s, "/merges/list", &json!({})).await?;
    let Some(n) = serde_json::from_value::<MergeQueue>(queue)
        .ok()
        .and_then(|q| q.holder(now()))
    else {
        return Ok(());
    };
    let (status, view) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    let Ok(c) = serde_json::from_value::<Change>(view["change"].clone()) else {
        return Ok(());
    };
    if status != 200 || merges::holds_turn(&c) {
        return Ok(());
    }
    let why = format!(
        "!{n} gave its merge turn back: {}",
        control::release_reason(&c)
    );
    let session = c.session.clone();
    let added = super::events::append(env, s, "merge.released", Some(n), session, &why, false);
    if let Err(e) = added.await {
        worker::console_error!("merge.released: {e}");
    }
    ended(env, &c, c.merged_as.as_deref()).await
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
        // One review per session and ADR: a stack of changes citing it shares one.
        let mut reviewed = std::collections::BTreeSet::new();
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
            if globs.is_empty()
                || crate::reconcile::generated(&open.slug)
                || !still_open(env, open.number).await?
            {
                continue;
            }
            let Some(p) =
                crate::reconcile::review(open.number, &open.plan, c.number, &c.plan, &globs)
            else {
                continue;
            };
            let session = reviewer(env, &s, open.number, open.session.clone())
                .await
                .unwrap_or_else(|e| {
                    worker::console_error!("reviewer of change {}: {e}", open.number);
                    open.session.clone()
                });
            if reviewed.insert((session.clone(), p.scope.clone()))
                && let Err(e) = reconcile(env, open.number, session, p).await
            {
                worker::console_error!("review of change {}: {e}", open.number);
            }
        }
    }
    let (path, body) = if merges::keeps_place(c) {
        ("/merges/yielded", json!({ "number": c.number }))
    } else if merged.is_some() {
        ("/merges/done", json!({ "number": c.number }))
    } else {
        ("/merges/left", left(c))
    };
    let (_, v) = to_instance(env, &s, path, &body).await?;
    start(env, &s, serde_json::from_value(v["next"].clone()).ok()).await
}

/// Who reviews open change `n` of `session`: its brief's planner when it carries a brief, which
/// an executor may not write; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md`, 2026-11-14.
///
/// # Errors
///
/// When a binding fails.
async fn reviewer(
    env: &Env,
    s: &Settings,
    n: u32,
    session: Option<String>,
) -> Result<Option<String>> {
    let (_, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    let Some(id) = v["change"]["brief"].as_str() else {
        return Ok(session);
    };
    let (status, brief) = to_instance(env, s, "/briefs/show", &json!({ "id": id })).await?;
    let planner = brief["planner"].as_str().filter(|_| status == 200);
    Ok(planner.map(str::to_string).or(session))
}

/// Whether change `n`, open in D1, is open in its own record; a merged or closed one is healed:
/// its D1 row and endings are run again.
///
/// # Errors
///
/// When a binding fails.
pub async fn still_open(env: &Env, n: u32) -> Result<bool> {
    let repo = Settings::from(env)?.repo;
    let (status, v) = to_change(env, &repo, n, Method::Get, "/status", None).await?;
    if status != 200 || !matches!(v["change"]["state"].as_str(), Some("merged" | "closed")) {
        return Ok(true);
    }
    let (status, v) = to_change(env, &repo, n, Method::Post, "/heal", Some(&json!({}))).await?;
    if status != 200 {
        worker::console_warn!("heal !{n}: {v}");
    }
    Ok(false)
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
    let open =
        control::open_rows(&platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?);
    if let Some(n) = crate::reconcile::existing(&open, &p.slug) {
        let why = format!("!{parent} conflicted again; it reuses !{n}, `{}`", p.slug);
        let added =
            super::events::append(env, &s, "reconcile.reused", Some(n), session, &why, false);
        if let Err(e) = added.await {
            worker::console_error!("reconcile.reused: {e}");
        }
        return Ok(Some(n));
    }
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
        super::briefs::by_window(env, &s, n, "change.authorize", None).await;
    }
    Ok(number)
}

/// Closes change `number`'s generated changes whose reason is gone, as
/// [`crate::reconcile::retired`] chooses; each never pushed, so the close loses nothing.
///
/// # Errors
///
/// When a binding fails.
pub async fn retire(env: &Env, number: u32, ended: bool) -> Result<()> {
    let s = Settings::from(env)?;
    let open =
        control::open_rows(&platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?);
    let close = serde_json::to_value(Event::Close)?;
    for n in crate::reconcile::retired(&open, number, ended) {
        let (status, v) = to_change(env, &s.repo, n, Method::Post, "/event", Some(&close)).await?;
        if status != 200 {
            worker::console_warn!("retire !{n}: {v}");
            continue;
        }
        super::tasks::close_for(env, &s, n).await?;
        let why = format!("!{n} closed: its reason on !{number} is gone");
        let added = super::events::append(env, &s, "reconcile.retired", Some(n), None, &why, false);
        if let Err(e) = added.await {
            worker::console_error!("reconcile.retired: {e}");
        }
    }
    Ok(())
}

async fn merge(env: &Env, s: &Settings, number: u32, by: &str) -> Result<(u16, Value)> {
    let body = serde_json::to_value(Event::Merge { by: by.to_string() })?;
    to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await
}

/// Change `n`'s own record, when it answers.
async fn record(env: &Env, s: &Settings, n: u32) -> Result<Option<Change>> {
    let (status, view) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    if status != 200 {
        return Ok(None);
    }
    Ok(serde_json::from_value(view["change"].clone()).ok())
}

/// The `/merges/left` body for change `c`, which gave its place back without merging.
fn left(c: &Change) -> Value {
    json!({ "number": c.number, "failed": merges::failed_at(c) })
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
        let body = record(env, s, e.number)
            .await?
            .map_or_else(|| json!({ "number": e.number }), |c| left(&c));
        let (_, v) = to_instance(env, s, "/merges/left", &body).await?;
        next = serde_json::from_value(v["next"].clone()).ok();
    }
    Ok(())
}

/// The `Instance` Durable Object's side: `/merges/enqueue`, `done`, `left`, `yielded`, `keep`, `release`, `front`, `resume`, `release-turn`, `release-turn-done` and `list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut q: MergeQueue = storage.get(MERGES).await?.unwrap_or_default();
    q.yielded = storage.get(super::YIELDS).await?.unwrap_or_default();
    let number = body["number"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_default();
    let (q, answer) = match path {
        "/merges/enqueue" => {
            let by = body["by"].as_str().unwrap_or_default();
            let docs = body["docs"].as_bool().unwrap_or(false);
            let back: Back = serde_json::from_value(body["back"].clone()).unwrap_or_default();
            let t = now();
            // A docs-only change takes the docs turn; any other a kept place back (ADR §8, §9).
            let (returned, (q, resumed, queued)) = if docs {
                (None, merges::ask_as(&q, number, by, true, t))
            } else {
                let returned = q.returning.get(&number).and_then(|r| r.returns(&back, t));
                (returned, merges::ask_back(&q, number, by, &back, t))
            };
            match queued {
                Ok(queued) => {
                    let returned = returned.filter(|_| matches!(queued, Queued::Behind(_)));
                    let answer =
                        json!({ "queued": queued, "resumed": resumed, "returned": returned });
                    (q, answer)
                }
                Err(why) => {
                    storage.put(MERGES, &q).await?;
                    let refused = json!({ "error": why, "resumed": resumed });
                    return Ok(Response::from_json(&refused)?.with_status(409));
                }
            }
        }
        "/merges/done" => {
            let (q, next) = merges::done(&q, number, now());
            (q, json!({ "next": next }))
        }
        "/merges/left" => {
            let failed = body["failed"].as_u64().and_then(|p| u32::try_from(p).ok());
            let (q, next) = merges::left(&q, number, failed, now());
            (q, json!({ "next": next }))
        }
        "/merges/front" => match merges::front(&q, number) {
            Ok((q, left)) => (q, json!({ "left": left })),
            Err(why) => {
                let refused = json!({ "error": why });
                return Ok(Response::from_json(&refused)?.with_status(409));
            }
        },
        "/merges/yielded" => {
            let (q, next) = merges::yielded_back(&q, number, now());
            (q, json!({ "next": next }))
        }
        "/merges/release" => {
            let (q, next) = merges::release(&q, number, now());
            (q, json!({ "next": next }))
        }
        "/merges/keep" => {
            let q = merges::keep(&q, number, now());
            let misses = q.kept.get(&number).map_or(1, |k| k.misses);
            (q, json!({ "misses": misses }))
        }
        "/merges/resume" => {
            let (q, next) = merges::advance(q, now());
            (q, json!({ "next": next }))
        }
        "/merges/release-turn" => {
            let waited = body["waited"].as_u64().unwrap_or_default();
            let (q, granted) = merges::release_turn(&q, waited, now());
            (q, json!({ "granted": granted }))
        }
        "/merges/release-turn-done" => {
            let (q, next) = merges::release_turn_done(&q, now());
            (q, json!({ "next": next }))
        }
        "/merges/list" => return Response::from_json(&q),
        _ => return Response::error("not found", 404),
    };
    storage.put(MERGES, &q).await?;
    Response::from_json(&answer)
}
