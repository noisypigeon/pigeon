//! A brief routed to a capacity pool: its route at `brief.create`, its dispatch in-process (label
//! ticket, task claim, proposal, the task opened in the `Pool` object, the pool job on `Checker`),
//! and what the job's report does; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`, its 2026-11-17
//! step 6 amendment.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use worker::{Env, Method, Result};

use super::mandates::to_instance;
use super::platform::{self, Settings, json_request, now};
use crate::control::{self, JobSpec, Report};
use crate::core::brief::Draft;
use crate::core::change::{Change, Event};
use crate::core::pool::config::{Budget, config};
use crate::core::pool::dispatch::{self as pd, Action, Secrets};
use crate::core::pool::ledger::Ledger;
use crate::core::pool::meter::{self, TaskInfo};
use crate::core::pool::route::{self, Route};

/// The proposer a pool task's change records.
pub const PROPOSER: &str = "pool:dispatch";

/// The harness pool tasks run under.
const HARNESS: &str = "claude-code";

/// A pool task's job, as the `Pool` object follows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Followed {
    /// The job's id on `Checker`.
    pub job: String,
    /// The task's change.
    pub change: u32,
    /// The task's label.
    pub label: String,
    /// When the job counts as lost.
    pub lost_at: u64,
}

/// The ledger pool `name`'s object keeps, if it has one yet.
async fn ledger_of(env: &Env, name: &str) -> Option<Ledger> {
    let (status, v) = super::pool::to_pool(env, name, "/status", &json!({}))
        .await
        .ok()?;
    (status == 200)
        .then(|| serde_json::from_value(v["ledger"].clone()).ok())
        .flatten()
}

/// Where brief `draft` runs, routed over every pool's ledger and the live executors.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn route(env: &Env, s: &Settings, draft: &Draft) -> Result<Route> {
    let cfg = match config() {
        Ok(c) => c,
        Err(why) => return Ok(Route::Session(format!("pool: {why}"))),
    };
    let budget = match pd::budget(&cfg, draft) {
        Ok(b) => b,
        Err(why) => return Ok(Route::Session(why)),
    };
    let at = now();
    let mut views = Vec::new();
    for name in &cfg.order {
        let Some(p) = cfg.pool(name) else { continue };
        let l = ledger_of(env, name).await;
        views.push(pd::view(p, l.as_ref(), draft.model, budget, at));
    }
    let (_, v) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(v).unwrap_or_default();
    Ok(route::route(
        draft.runs_on,
        draft.model,
        HARNESS,
        &cfg,
        &views,
        pd::executors(&tickets, at),
        at,
    ))
}

fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

/// Ends `label`'s ticket, logging a refusal.
async fn end_label(env: &Env, s: &Settings, label: &str) {
    if let Err(e) = super::tickets::end(env, s, &args(&[("label", json!(label))])).await {
        worker::console_error!("pool dispatch: {label} didn't end: {e}");
    }
}

/// Dispatches brief `brief` (task `task`, planned by `planner`) to pool `pool`, in the ADR's
/// order; a failure part-way ends the label, closes the task, and answers why.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn dispatch(
    env: &Env,
    s: &Settings,
    brief: &str,
    draft: &Draft,
    task: u64,
    planner: &str,
    pool: &str,
) -> Result<std::result::Result<Value, String>> {
    let (_, v) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(v).unwrap_or_default();
    let k = pd::next_k(tickets.0.keys().map(String::as_str), brief);
    let label = pd::label(brief, k);
    let task_id = pd::task_id(brief, k);
    let started = super::tickets::start(
        env,
        s,
        &args(&[
            ("label", json!(label)),
            ("model", json!(draft.model.name())),
            ("planner", json!(planner)),
            ("brief", json!(brief)),
            ("parent", json!(planner)),
        ]),
    )
    .await?;
    if started.0 != 200 {
        return Ok(Err(format!("pool: {label}'s ticket: {}", started.1)));
    }
    match open(env, s, brief, draft, task, &label, &task_id, pool).await? {
        Ok(v) => Ok(Ok(v)),
        Err(why) => {
            end_label(env, s, &label).await;
            let close = json!({ "task": task_id, "outcome": "failed" });
            let _ = super::pool::to_pool(env, pool, "/task/close", &close).await;
            Ok(Err(why))
        }
    }
}

/// Routes each waiting `runs_on: pool` brief again, oldest first, dispatching under its planner,
/// until one still waits; see the capacity pools ADR's 2026-11-17 step 6 amendment.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn reroute(env: &Env, s: &Settings) -> Result<()> {
    let (_, v) = to_instance(env, s, "/tasks/waiting-briefs", &json!({})).await?;
    let waiting: Vec<(u32, String)> =
        serde_json::from_value(v["briefs"].clone()).unwrap_or_default();
    for (task, brief) in waiting {
        let (_, shown) = to_instance(env, s, "/briefs/show", &json!({ "id": brief })).await?;
        let draft = shown["revisions"]
            .as_array()
            .and_then(|r| r.last())
            .and_then(|r| serde_json::from_value::<Draft>(r["draft"].clone()).ok());
        let (Some(draft), Some(planner)) = (draft, shown["planner"].as_str()) else {
            continue;
        };
        if draft.runs_on != route::RunsOn::Pool {
            continue;
        }
        let said = match route(env, s, &draft).await? {
            Route::Pool(pool) => {
                match dispatch(env, s, &brief, &draft, u64::from(task), planner, &pool).await? {
                    Ok(d) => format!("{brief} dispatched to pool {pool}: {d}"),
                    Err(why) => format!("{brief} wasn't dispatched: {why}"),
                }
            }
            Route::Wait(_) => break,
            Route::Session(why) => format!("{brief} goes to a session: {why}"),
        };
        super::events::append(
            env,
            s,
            "brief.routed",
            None,
            Some(planner.to_string()),
            &said,
            false,
        )
        .await?;
    }
    Ok(())
}

/// The steps after the label's ticket: claim, proposal, the task opened, the job started.
#[allow(clippy::too_many_arguments)]
async fn open(
    env: &Env,
    s: &Settings,
    brief: &str,
    draft: &Draft,
    task: u64,
    label: &str,
    task_id: &str,
    pool: &str,
) -> Result<std::result::Result<Value, String>> {
    let claim = args(&[("session", json!(label)), ("id", json!(task))]);
    let (status, v) = super::tasks::run(env, s, "task.claim", &claim).await?;
    if status != 200 {
        return Ok(Err(format!(
            "pool: {label} couldn't claim task {task}: {v}"
        )));
    }
    let number = match propose(env, s, brief, label, task_id).await? {
        Ok(n) => n,
        Err(why) => return Ok(Err(why)),
    };
    let cfg = config().map_err(worker::Error::RustError)?;
    let (budget, model) = match (pd::budget(&cfg, draft), pd::model(draft.model)) {
        (Ok(b), Ok(m)) => (b, m),
        (Err(why), _) | (_, Err(why)) => return Ok(Err(why)),
    };
    let info = TaskInfo {
        label: label.to_string(),
        change: u64::from(number),
        brief: brief.to_string(),
        class: draft.model.name().to_string(),
        model: model.clone(),
    };
    let body = json!({ "pool": pool, "task": task_id, "info": info, "budget": budget });
    let (status, v) = super::pool::to_pool(env, pool, "/task/open", &body).await?;
    if status != 200 {
        return Ok(Err(format!("pool: the task didn't open: {v}")));
    }
    let opened = now();
    let spec = match spec(
        env, s, brief, draft, number, task_id, pool, &model, &budget, opened,
    )
    .await?
    {
        Ok(spec) => spec,
        Err(why) => return Ok(Err(why)),
    };
    let req = json_request(
        "https://checker/start",
        Method::Post,
        Some(&serde_json::to_value(&spec)?),
        None,
    )?;
    platform::job_stub(env, control::class_of(control::POOL), &spec.id)?
        .fetch_with_request(req)
        .await?;
    let followed = Followed {
        job: spec.id.clone(),
        change: number,
        label: label.to_string(),
        lost_at: pd::lost_at(opened, &budget),
    };
    let body = json!({ "task": task_id, "followed": followed });
    super::pool::to_pool(env, pool, "/task/job", &body).await?;
    Ok(Ok(json!({
        "pool": pool, "task": task_id, "label": label, "change": number, "job": spec.id,
    })))
}

/// Proposes brief `brief`'s change as `label`, bound to the brief and authorized by the window.
async fn propose(
    env: &Env,
    s: &Settings,
    brief: &str,
    label: &str,
    task_id: &str,
) -> Result<std::result::Result<u32, String>> {
    let (title, plan, scope, id) = match super::executes(
        env,
        s,
        Some(label),
        Some(brief.to_string()),
        &[],
        &[],
        None,
    )
    .await?
    {
        Ok(Some(found)) => found,
        Ok(None) => return Ok(Err(format!("pool: {brief} gave no title, plan or scope"))),
        Err(why) => return Ok(Err(why)),
    };
    let slug = format!("pool-{}", task_id.replace('.', "-"));
    let mut change = Change::propose(0, &slug, &title, &plan, scope, PROPOSER);
    change.session = Some(label.to_string());
    change.brief = Some(id.clone());
    let (number, answer) = super::propose(env, s, "change.propose", change).await?;
    let Some(number) = number.and_then(|n| n.parse::<u32>().ok()) else {
        return Ok(Err(format!("pool: the change wasn't proposed: {answer:?}")));
    };
    to_instance(
        env,
        s,
        "/briefs/bind",
        &json!({ "id": id, "change": number }),
    )
    .await?;
    super::briefs::by_window(env, s, number, "change.authorize", None).await;
    Ok(Ok(number))
}

/// The pool job's spec: the change's branch at the brief's base, a write token, and its env.
#[allow(clippy::too_many_arguments)]
async fn spec(
    env: &Env,
    s: &Settings,
    brief: &str,
    draft: &Draft,
    number: u32,
    task_id: &str,
    pool: &str,
    model: &str,
    budget: &Budget,
    opened: u64,
) -> Result<std::result::Result<JobSpec, String>> {
    let (_, shown) = to_instance(env, s, "/briefs/show", &json!({ "id": brief })).await?;
    let Some(latest) = shown["revisions"]
        .as_array()
        .and_then(|r| r.last())
        .map(|r| r["draft"].clone())
    else {
        return Ok(Err(format!("pool: {brief} has no revision")));
    };
    let (_, view) = super::to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
    let Ok(change) = serde_json::from_value::<Change>(view["change"].clone()) else {
        return Ok(Err(format!("pool: !{number} can't be read")));
    };
    let key = meter::task_key(platform::secret(env, "SESSION_KEY")?.as_bytes());
    let token = meter::sign_task(&key, pool, task_id, pd::token_expiry(opened, budget));
    let (Ok(access_id), Ok(access_secret)) = (
        platform::secret(env, super::pool::ACCESS_ID_VAR),
        platform::secret(env, ACCESS_SECRET_VAR),
    ) else {
        return Ok(Err("pool: the pool's Access token isn't set".into()));
    };
    let host = env
        .var(crate::wire::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    let job_env = pd::job_env(
        task_id,
        &pd::pool_url(&host, pool),
        model,
        &latest,
        budget,
        &Secrets {
            task_token: &token,
            access_id: &access_id,
            access_secret: &access_secret,
        },
    );
    let (remote, write) =
        platform::credentials(env, true, crate::wire::artifacts_wire::TOKEN_TTL).await?;
    Ok(Ok(JobSpec {
        id: format!("pool-{}", task_id.replace('.', "-")),
        branch: change.branch(),
        number,
        title: change.title.clone(),
        plan: change.plan.clone(),
        scope: change.scope.clone(),
        job: control::POOL.into(),
        ..control::deploy_spec(&remote, &write, &draft.base, None, job_env)
    }))
}

/// The Worker secret holding the pool's Access service token's secret.
pub const ACCESS_SECRET_VAR: &str = "POOL_ACCESS_SECRET";

/// The job's report: `Some(None)` while it runs, `Some(Some(report))` once it ended, and none
/// when `Checker` no longer knows it.
pub(super) async fn report(env: &Env, job: &str) -> Result<Option<Option<Report>>> {
    let req = json_request(
        &format!("https://checker/status/{job}"),
        Method::Get,
        None,
        None,
    )?;
    let mut r = platform::job_stub(env, control::class_of(control::POOL), job)?
        .fetch_with_request(req)
        .await?;
    match r.status_code() {
        200 => {
            let report: Report = r.json().await?;
            Ok(Some(
                (report.outcome != control::Outcome::Running).then_some(report),
            ))
        }
        404 => Ok(None),
        _ => Ok(Some(None)),
    }
}

/// What a finished or lost job does: its change hears the push, or the planner hears an
/// escalation; then the label ends. Answers the task's close outcome.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn act(
    env: &Env,
    s: &Settings,
    followed: &Followed,
    report: Option<&Report>,
) -> Result<String> {
    let (action, outcome) = pd::action(report.and_then(|r| r.pool.as_ref()));
    match action {
        Action::Pushed { head } => {
            let body = serde_json::to_value(Event::Pushed { sha: head })?;
            let _ = super::to_change(
                env,
                &s.repo,
                followed.change,
                Method::Post,
                "/event",
                Some(&body),
            )
            .await;
        }
        Action::Escalate { question, tried } => {
            let a = args(&[
                ("session", json!(followed.label)),
                ("change", json!(followed.change)),
                ("question", json!(question)),
                ("tried", json!(tried)),
            ]);
            let (status, v) = super::briefs::escalate(env, s, &a).await?;
            if status != 200 {
                worker::console_error!("pool: !{} didn't escalate: {v}", followed.change);
            }
        }
    }
    end_label(env, s, &followed.label).await;
    Ok(outcome)
}
