//! Briefs in the control plane: the `brief.*` operations and the `Instance` Durable Object's
//! store of them; see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::briefs::Briefs;
use crate::core::brief::Draft;
use crate::sessions::Class;
use crate::tickets::Tickets;

/// The storage key of every brief.
const BRIEFS: &str = "briefs";

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The `draft` argument: an object, or one as JSON text.
fn draft(args: &Map<String, Value>) -> std::result::Result<Draft, String> {
    let v = match args.get("draft") {
        Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| e.to_string())?,
        Some(v) => v.clone(),
        None => return Err("`draft` is the brief's content".into()),
    };
    serde_json::from_value(v).map_err(|e| format!("`draft`: {e}"))
}

/// `session`, when its ticket's class plans: opus, or a label never started.
async fn planner(
    env: &Env,
    s: &Settings,
    args: &Map<String, Value>,
) -> Result<std::result::Result<String, String>> {
    let Some(label) = text(args, "session") else {
        return Ok(Err("`session` is the planner's label".into()));
    };
    let (_, t) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: Tickets = serde_json::from_value(t).unwrap_or_default();
    let class = tickets.class(&label).unwrap_or_default();
    Ok(if class == Class::Opus {
        Ok(label)
    } else {
        Err(format!(
            "session {label} is {}, an executor; only an opus session writes briefs",
            class.name()
        ))
    })
}

/// Runs `operation` with `args`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    match operation {
        "brief.show" => {
            to_instance(env, s, "/briefs/show", &json!({ "id": text(args, "id") })).await
        }
        "brief.list" => {
            let body = json!({ "planner": text(args, "planner") });
            to_instance(env, s, "/briefs/list", &body).await
        }
        "brief.create" | "brief.amend" => {
            let label = match planner(env, s, args).await? {
                Ok(l) => l,
                Err(why) => return Ok((403, json!(why))),
            };
            let d = match draft(args) {
                Ok(d) => d,
                Err(why) => return Ok((400, json!(why))),
            };
            let (path, kind) = if operation == "brief.create" {
                ("/briefs/create", "brief.created")
            } else {
                ("/briefs/amend", "brief.amended")
            };
            let body = json!({
                "planner": label, "draft": d, "id": text(args, "id"), "reason": text(args, "reason"),
            });
            let (status, mut v) = to_instance(env, s, path, &body).await?;
            if status == 200 && operation == "brief.create" {
                let id = v["id"].as_str().unwrap_or_default().to_string();
                let note = format!("{id} ({}): {}", d.model.name(), d.title);
                let task = json!({ "brief": id, "model": d.model, "note": note });
                let (_, added) = to_instance(env, s, "/tasks/add-brief", &task).await?;
                // Where it runs; see the capacity pools ADR's 2026-11-17 step 6 amendment.
                let route = super::pool_dispatch::route(env, s, &d).await?;
                v["route"] = serde_json::to_value(&route)?;
                if let (crate::core::pool::route::Route::Pool(pool), Some(task)) =
                    (&route, added["id"].as_u64())
                {
                    v["dispatch"] =
                        match super::pool_dispatch::dispatch(env, s, &id, &d, task, &label, pool)
                            .await?
                        {
                            Ok(d) => d,
                            Err(why) => json!({ "refused": why }),
                        };
                }
            }
            if status == 200 {
                let said = format!(
                    "{} r{} ({}): {}",
                    v["id"].as_str().unwrap_or_default(),
                    v["revisions"].as_array().map_or(0, Vec::len),
                    d.model.name(),
                    d.title
                );
                super::events::append(env, s, kind, None, Some(label.clone()), &said, false)
                    .await?;
            }
            if status == 200 && operation == "brief.amend" {
                let changes: Vec<u32> =
                    serde_json::from_value(v["changes"].clone()).unwrap_or_default();
                let id = v["id"].as_str().unwrap_or_default();
                let r = v["revisions"].as_array().map_or(0, Vec::len);
                for n in changes {
                    reopen(
                        env,
                        s,
                        n,
                        &label,
                        &format!("{id} r{r} answers the escalation"),
                    )
                    .await?;
                }
            }
            Ok((status, v))
        }
        "brief.review" => review(env, s, args).await,
        "brief.escalate" => escalate(env, s, args).await,
        other => Ok((404, json!(format!("`{other}` is not a brief operation")))),
    }
}

/// `brief.review`: the brief's planner accepts, revises or takes back an executor's change; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §5.
async fn review(env: &Env, s: &Settings, args: &Map<String, Value>) -> Result<(u16, Value)> {
    let label = match planner(env, s, args).await? {
        Ok(l) => l,
        Err(why) => return Ok((403, json!(why))),
    };
    let Some(n) = text(args, "change").and_then(|c| c.trim_start_matches('!').parse::<u32>().ok())
    else {
        return Ok((400, json!("`change` is the executor's change number")));
    };
    let (_, view) = super::to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    let Some(id) = view["change"]["brief"].as_str().map(str::to_string) else {
        return Ok((409, json!(format!("!{n} carries out no brief"))));
    };
    let (_, brief) = to_instance(env, s, "/briefs/show", &json!({ "id": id })).await?;
    if brief["planner"].as_str() != Some(label.as_str()) {
        return Ok((403, json!(format!("only {id}'s planner reviews !{n}"))));
    }
    let notes = text(args, "notes").unwrap_or_default();
    let executor = view["change"]["session"].as_str().map(str::to_string);
    let verdict = text(args, "verdict").unwrap_or_default();
    let said = match verdict.as_str() {
        "accept" => {
            let Some(head) = text(args, "head") else {
                return Ok((400, json!("`head` is the head reviewed")));
            };
            let body = json!({ "type": "accept", "by": label, "sha": head });
            let (status, v) =
                super::to_change(env, &s.repo, n, Method::Post, "/event", Some(&body)).await?;
            if status != 200 {
                return Ok((status, v));
            }
            if v["state"] == "checked" {
                by_window(env, s, n, "change.approve", Some(&head)).await;
            }
            format!("!{n}'s head {head} is accepted against {id}")
        }
        "revise" => format!("{id}'s planner asks !{n} for a revision: {notes}"),
        "take" => {
            let note = format!("!{n} is taken back by {id}'s planner: {notes}");
            super::tasks::add(env, s, "handoff", n, None, Some(label.clone()), &note).await?;
            reopen(env, s, n, &label, &note).await?;
            // Its executor's session ends; see the archive ADR's 2026-11-18 amendment.
            if let Some(executor) = executor.as_deref()
                && let Err(e) = super::tickets::brief_done(env, s, executor, &id, n, false).await
            {
                worker::console_error!("end of !{n}'s executor: {e}");
            }
            note
        }
        _ => return Ok((400, json!("`verdict` is accept, revise or take"))),
    };
    let kind = if verdict == "revise" {
        "change.note"
    } else {
        "brief.reviewed"
    };
    super::events::append(env, s, kind, Some(n), executor, &said, false).await?;
    let by = crate::core::brief::report::Tally {
        reviews: 1,
        revised: u32::from(verdict == "revise"),
        ..Default::default()
    };
    count(env, s, &id, by).await;
    let body = json!({ "session": label, "class": Class::Opus });
    let (_, open) = to_instance(env, s, "/tasks/list", &body).await?;
    for t in open["tasks"].as_array().into_iter().flatten() {
        if t["kind"] == "brief-review" && t["change"].as_u64() == Some(u64::from(n)) {
            to_instance(env, s, "/tasks/done", &json!({ "id": t["id"] })).await?;
        }
    }
    Ok((200, json!({ "change": n, "brief": id, "verdict": verdict })))
}

/// `brief.escalate`: the executor of change `change` asks its brief's planner `question`, having
/// `tried`; its branch takes no push until the planner answers; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §6.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn escalate(
    env: &Env,
    s: &Settings,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    let Some(label) = text(args, "session") else {
        return Ok((400, json!("`session` is the executor's label")));
    };
    let Some(n) = text(args, "change").and_then(|c| c.trim_start_matches('!').parse::<u32>().ok())
    else {
        return Ok((400, json!("`change` is your change number")));
    };
    let Some(question) = text(args, "question") else {
        return Ok((400, json!("`question` says what the brief doesn't settle")));
    };
    let (_, view) = super::to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    if view["change"]["session"].as_str() != Some(label.as_str()) {
        return Ok((403, json!(format!("!{n} isn't session {label}'s"))));
    }
    let Some(id) = view["change"]["brief"].as_str().map(str::to_string) else {
        return Ok((409, json!(format!("!{n} carries out no brief"))));
    };
    let asked = match text(args, "tried") {
        Some(t) => format!("{question} (tried: {t})"),
        None => question,
    };
    let body = json!({ "type": "escalate", "by": label, "question": asked, "at": now() });
    let (status, v) =
        super::to_change(env, &s.repo, n, Method::Post, "/event", Some(&body)).await?;
    if status != 200 {
        return Ok((status, v));
    }
    let (_, brief) = to_instance(env, s, "/briefs/show", &json!({ "id": id })).await?;
    let planner = brief["planner"].as_str().map(str::to_string);
    let note = format!("!{n} escalates to {id}'s planner: {asked}");
    super::tasks::add(env, s, "escalation", n, None, planner, &note).await?;
    let by = crate::core::brief::report::Tally {
        escalations: 1,
        ..Default::default()
    };
    count(env, s, &id, by).await;
    Ok((200, json!({ "change": n, "brief": id, "escalated": asked })))
}

/// Runs `operation` on brief change `n` (at `sha`, for an approval) as the owner's Auto window,
/// when one covers it, with no agent asking; a refusal is logged and leaves the stage to say who
/// acts. See the 2026-11-14 amendment on the window authorizing and approving a brief's change.
pub(super) async fn by_window(env: &Env, s: &Settings, n: u32, operation: &str, sha: Option<&str>) {
    let mut args = Map::new();
    args.insert("number".into(), json!(n));
    if let Some(sha) = sha {
        args.insert("sha".into(), json!(sha));
    }
    let request = crate::wire::Request {
        operation: operation.into(),
        args,
    };
    let ran = match super::asks_owner(env, s, n, &request).await {
        Ok(true) => return,
        Ok(false) => Box::pin(super::under_window(env, s, n, &request)).await,
        Err(e) => Err(e),
    };
    match ran {
        Ok(None | Some(Ok(_))) => {}
        Ok(Some(Err(e))) => worker::console_warn!("`{operation}` on !{n} by window: {e}"),
        Err(e) => worker::console_error!("`{operation}` on !{n} by window: {e}"),
    }
}

/// Clears change `n`'s open escalation, as its brief's planner `by`, telling its executor `why`.
async fn reopen(env: &Env, s: &Settings, n: u32, by: &str, why: &str) -> Result<()> {
    let (_, view) = super::to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    if view["change"]["escalated"].is_null() {
        return Ok(());
    }
    let body = json!({ "type": "reopen", "by": by });
    super::to_change(env, &s.repo, n, Method::Post, "/event", Some(&body)).await?;
    let executor = view["change"]["session"].as_str().map(str::to_string);
    super::events::append(env, s, "change.note", Some(n), executor, why, false).await?;
    let body = json!({ "session": by, "class": Class::Opus });
    let (_, open) = to_instance(env, s, "/tasks/list", &body).await?;
    for t in open["tasks"].as_array().into_iter().flatten() {
        if t["kind"] == "escalation" && t["change"].as_u64() == Some(u64::from(n)) {
            to_instance(env, s, "/tasks/done", &json!({ "id": t["id"] })).await?;
        }
    }
    Ok(())
}

/// Adds `by` to brief `id`'s tally; a failure is logged, never answered, as the count is only
/// the digest's.
pub async fn count(env: &Env, s: &Settings, id: &str, by: crate::core::brief::report::Tally) {
    let body = json!({ "id": id, "tally": by });
    if let Err(e) = to_instance(env, s, "/briefs/tally", &body).await {
        worker::console_error!("tally for {id}: {e}");
    }
}

/// Every brief, newest first; none when they can't be read.
pub async fn all(env: &Env, s: &Settings) -> Vec<crate::core::brief::Brief> {
    match to_instance(env, s, "/briefs/all", &json!({})).await {
        Ok((200, v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Brief `id`'s latest draft; none when there is no such brief, or it can't be read.
pub async fn latest(env: &Env, s: &Settings, id: &str) -> Option<Draft> {
    let (status, brief) = to_instance(env, s, "/briefs/show", &json!({ "id": id }))
        .await
        .ok()?;
    let brief: crate::core::brief::Brief = serde_json::from_value(brief)
        .ok()
        .filter(|_| status == 200)?;
    Some(brief.latest().draft.clone())
}

/// When `after` carries a brief and a head newly checked awaits review: a `brief-review` task
/// for its planner, which wakes its follower.
///
/// # Errors
///
/// When a binding fails.
pub async fn review_needed(
    env: &Env,
    s: &Settings,
    before: Option<&crate::core::change::Change>,
    after: &crate::core::change::Change,
) -> Result<()> {
    use crate::core::change::State as Stage;
    let newly = after.state == Stage::Checked
        && before.is_none_or(|b| b.state != Stage::Checked || b.head != after.head);
    let Some(id) = after
        .brief
        .as_deref()
        .filter(|_| newly && after.awaits_review())
    else {
        return Ok(());
    };
    let (status, brief) = to_instance(env, s, "/briefs/show", &json!({ "id": id })).await?;
    if status != 200 {
        return Ok(());
    }
    let planner = brief["planner"].as_str().map(str::to_string);
    let note = format!(
        "review !{} ({}) against {id}: `brief_review` with accept, revise or take",
        after.number, after.title
    );
    super::tasks::add(env, s, "brief-review", after.number, None, planner, &note).await
}

/// Brief `n` as `brief_archive` holds it, if it does.
async fn archived(env: &Env, n: u32) -> Option<crate::core::brief::Brief> {
    let q = crate::control::Statement {
        sql: "SELECT body FROM brief_archive WHERE number = ?1",
        params: vec![json!(n)],
    };
    let rows = super::platform::d1_rows(env, &q).await.ok()?;
    let body = rows.first()?["body"].as_str()?;
    serde_json::from_str(body).ok()
}

/// Moves each finished brief of `all` into `brief_archive`: the rows first, in one batch, and
/// only then out of `all`; a failure leaves every brief where it was. See the briefs ADR's
/// 2026-11-20 archive amendment.
async fn archive_finished(env: &Env, all: &mut Briefs) {
    let Ok(s) = Settings::from(env) else {
        return;
    };
    let Ok(rows) = super::platform::d1_rows(env, &crate::control::open(&s.instance, &s.repo)).await
    else {
        return;
    };
    let open: std::collections::BTreeSet<u32> = rows
        .iter()
        .filter_map(|r| r["number"].as_u64().and_then(|n| u32::try_from(n).ok()))
        .collect();
    let done = all.finished(&open, now());
    let statements: Vec<crate::control::Statement> = done
        .iter()
        .filter_map(|n| {
            let b = all.briefs.get(n)?;
            Some(crate::control::Statement {
                sql: "INSERT OR REPLACE INTO brief_archive (number, body, archived_at) VALUES (?1, ?2, ?3)",
                params: vec![json!(n), json!(serde_json::to_string(b).ok()?), json!(now())],
            })
        })
        .collect();
    if statements.is_empty() {
        return;
    }
    if let Err(e) = super::platform::d1(env, &statements).await {
        worker::console_warn!("briefs: archiving failed: {e}");
        return;
    }
    for n in done {
        all.take(n);
    }
}

/// The `Instance` Durable Object's side: `/briefs/create`, `/briefs/amend`, `/briefs/show`,
/// `/briefs/list`, `/briefs/all` and `/briefs/tally`. A finished brief lives in
/// `brief_archive`: `show` reads it there, and a write to it puts it back first.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, env: &Env, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Briefs = storage.get(BRIEFS).await?.unwrap_or_default();
    let planner = body["planner"].as_str().unwrap_or_default();
    let id = body["id"].as_str().unwrap_or_default();
    if all.get(id).is_none()
        && let Some(n) = crate::briefs::number(id)
        && let Some(b) = archived(env, n).await
    {
        if path == "/briefs/show" {
            return Response::from_json(&b);
        }
        all.put_back(b);
    }
    let draft =
        || serde_json::from_value::<Draft>(body["draft"].clone()).map_err(|e| vec![e.to_string()]);
    let answer = match path {
        "/briefs/show" => {
            return match all.get(id) {
                Some(b) => Response::from_json(b),
                None => Response::error(format!("no brief `{id}`"), 404),
            };
        }
        "/briefs/list" => {
            let p = body["planner"].as_str();
            let rows: Vec<Value> = all
                .list(p)
                .into_iter()
                .map(|b| {
                    let r = b.latest();
                    json!({
                        "id": b.id, "planner": b.planner, "revision": b.revision(),
                        "model": r.draft.model, "title": r.draft.title,
                        "adr": r.draft.adr, "step": r.draft.step, "at": r.at,
                    })
                })
                .collect();
            return Response::from_json(&json!({ "briefs": rows }));
        }
        "/briefs/all" => {
            let every: Vec<&crate::core::brief::Brief> = all.list(None);
            return Response::from_json(&every);
        }
        "/briefs/tally" => {
            let by = serde_json::from_value(body["tally"].clone()).unwrap_or_default();
            all.tally(id, by).cloned().map_err(|e| vec![e])
        }
        "/briefs/create" => draft().and_then(|d| all.create(planner, d, now())),
        "/briefs/bind" => {
            let change = body["change"]
                .as_u64()
                .and_then(|c| u32::try_from(c).ok())
                .unwrap_or_default();
            all.bind(id, change).cloned().map_err(|e| vec![e])
        }
        "/briefs/amend" => {
            let reason = body["reason"].as_str().unwrap_or_default();
            draft().and_then(|d| all.amend(id, planner, d, reason, now()))
        }
        _ => return Response::error("not found", 404),
    };
    match answer {
        Ok(b) => {
            archive_finished(env, &mut all).await;
            storage.put(BRIEFS, &all).await?;
            Response::from_json(&b)
        }
        Err(problems) => Response::error(problems.join("; "), 422),
    }
}
