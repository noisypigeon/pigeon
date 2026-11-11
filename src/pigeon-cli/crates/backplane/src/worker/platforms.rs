//! Platform runs in the control plane: `platform.plan`, `platform.apply` and
//! `platform.status`; each step granted on `/platform/<id>` with the owner's code, the plan's
//! page also taking a one-time account token, sealed under `BACKPLANE_TOKEN` and kept in the
//! `MainRun` object only while the run needs it; the jobs run on a Runner, polled by `MainRun`'s
//! alarm; see `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's
//! amendment.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::approve as page;
use super::elicit;
use super::platform::{self, Settings, job_stub, json_request, now, secret};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::control::{self, JobSpec, Outcome, Report};
use crate::platforms::{Kept, Run, Runs, Stage};

const RUNS: &str = "platform-runs";

/// Where `MainRun` keeps each platform unit's newest outputs.
const OUTPUTS: &str = "platform-outputs";

/// How often a running platform job is asked how it went.
pub const POLL: Duration = Duration::from_secs(15);

/// How many times a lost platform job is started again before its run fails.
const ATTEMPTS: u32 = 3;

fn host(env: &Env) -> String {
    env.var(crate::cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default()
}

fn main_stub(env: &Env, s: &Settings) -> Result<worker::Stub> {
    env.durable_object("MAIN_RUN")?
        .id_from_name(&format!("{}#main", s.repo))?
        .get_stub()
}

/// Calls `MainRun`'s platform `path` with `body`: its status and JSON.
async fn to_main(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<(u16, Value)> {
    let req = json_request(
        &format!("https://main/platform/{path}"),
        Method::Post,
        Some(body),
        None,
    )?;
    let mut r = platform::call(env, &main_stub(env, s)?, req).await?;
    let status = r.status_code();
    // A refusal is text, as `Response::error` writes it.
    let text = r.text().await.unwrap_or_default();
    let v = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok((status, v))
}

/// Puts a run's pending grant in the inbox, pushes it to the owner, and answers its link.
async fn announce(env: &Env, s: &Settings, run: &Run, operation: &str, token: &str) -> String {
    let path = format!("/platform/{}?grant={token}", run.id);
    let waiting = Waiting {
        number: run.id,
        operation: operation.into(),
        title: run.unit.clone(),
        path: path.clone(),
        expires: run.offer_expires,
    };
    if let Err(e) = elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await {
        worker::console_error!("platform inbox: {e}");
    }
    let said = format!(
        "`{operation}` of {} (run {}) waits for the owner",
        run.unit, run.id
    );
    let _ = super::events::append(env, s, "approval.pending", None, None, &said, false).await;
    super::push::notify(env, s, "Approval waiting", &said, &path).await;
    format!("https://{}{path}", host(env))
}

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    args.get(k).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// `platform.plan`, `platform.apply` and `platform.status`; each answers at once, a grant
/// waiting on its page.
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
        "platform.plan" => {
            let token = platform::random_hex(32)?;
            let body = json!({ "unit": text(args, "unit"), "token": token, "now": now() });
            let (status, v) = to_main(env, s, "propose", &body).await?;
            if status != 200 {
                return Ok((status, v));
            }
            let run: Run = serde_json::from_value(v)?;
            let link = announce(env, s, &run, "platform.plan", &token).await;
            Ok((
                202,
                json!({ "run": run.shown(),
                "pending": { "operation": "platform.plan", "link": link } }),
            ))
        }
        "platform.apply" => {
            let token = platform::random_hex(32)?;
            let id = text(args, "run").and_then(|i| i.parse::<u32>().ok());
            let body = json!({ "id": id, "token": token, "now": now() });
            let (status, v) = to_main(env, s, "reoffer", &body).await?;
            if status != 200 {
                return Ok((status, v));
            }
            let run: Run = serde_json::from_value(v)?;
            let link = announce(env, s, &run, "platform.apply", &token).await;
            Ok((
                202,
                json!({ "run": run.shown(),
                "pending": { "operation": "platform.apply", "link": link } }),
            ))
        }
        "platform.status" => to_main(env, s, "list", &json!({})).await,
        other => Ok((404, json!(format!("`{other}` is not a platform operation")))),
    }
}

/// Whether the inbox's grant `body` (its token) is still run `id`'s pending one.
///
/// # Errors
///
/// When the binding fails.
pub async fn live(env: &Env, s: &Settings, id: u32, body: &Value) -> Result<bool> {
    let mut b = body.clone();
    b["id"] = json!(id);
    Ok(to_main(env, s, "peek", &b).await?.0 == 200)
}

fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/platform/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Checks the owner's account token: active, and on this platform's one account.
async fn verified(token: &str) -> std::result::Result<(), String> {
    let account = platform::account(token).await.map_err(|_| {
        "the token must see exactly one account (Account Settings Read)".to_string()
    })?;
    let v = platform::cloudflare(
        token,
        Method::Get,
        &format!("/accounts/{account}/tokens/verify"),
        None,
    )
    .await
    .map_err(|_| "the token isn't an active account token".to_string())?;
    if v["status"].as_str() == Some("active") {
        Ok(())
    } else {
        Err("the token isn't active".into())
    }
}

/// Serves `/platform/<id>` for `who`: the page of its pending grant, and the POST that grants or
/// refuses it.
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
        return page::refused(
            403,
            "only a person signed in to Access grants a platform run",
        );
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
            let (status, v) = to_main(env, s, "peek", &json!({ "id": id, "token": token })).await?;
            match serde_json::from_value::<Run>(v.clone()) {
                Ok(run) if status == 200 => page::html(200, &approve::platform_page(&run, &token)),
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
            let path = format!("/platform/{id}?grant={token}");
            let decided = decide(env, s, who, id, &token, &form).await;
            let _ = elicit::inbox(env, s, "/approvals/remove", &json!({ "path": path })).await;
            decided
        }
        _ => page::refused(405, "method not allowed"),
    }
}

async fn decide(
    env: &Env,
    s: &Settings,
    who: &Identity,
    id: u32,
    token: &str,
    form: &str,
) -> Result<Response> {
    let (status, v) = to_main(env, s, "peek", &json!({ "id": id, "token": token })).await?;
    let Ok(run) = serde_json::from_value::<Run>(v.clone()) else {
        return page::refused(403, v.as_str().unwrap_or("this link is not valid"));
    };
    if status != 200 {
        return page::refused(403, "this link is not valid");
    }
    if approve::field(form, "decision").as_deref() == Some("refuse") {
        let _ = to_main(env, s, "refuse", &json!({ "id": id, "by": who.subject })).await?;
        let said = format!("platform run {id} of {} was refused", run.unit);
        let _ = super::events::append(env, s, "platform.failed", None, None, &said, false).await;
        return page::html(200, &approve::outcome("Refused", &said));
    }
    let code = approve::field(form, "code").unwrap_or_default();
    if let Err(why) = elicit::check_code(env, s, code.trim()).await? {
        return page::refused(403, &why);
    }
    let (path, body) = if run.stage == Stage::Offered {
        let account = approve::field(form, "account_token").unwrap_or_default();
        let account = account.trim();
        if let Err(why) = verified(account).await {
            return page::refused(403, &why);
        }
        let key = secret(env, crate::cobot::worker::TOKEN_VAR)?;
        let sealed = platform::seal(&key, account).await?;
        (
            "grant-plan",
            json!({ "id": id, "token": token, "sealed": sealed, "by": who.subject, "now": now() }),
        )
    } else {
        (
            "grant-apply",
            json!({ "id": id, "token": token, "by": who.subject, "now": now() }),
        )
    };
    let (status, v) = to_main(env, s, path, &body).await?;
    match serde_json::from_value::<Run>(v.clone()) {
        Ok(r) if status == 200 => page::html(
            200,
            &approve::outcome(
                &format!("Platform run {} granted", r.id),
                &format!(
                    "{} {}: the job runs now. Recorded as {}.",
                    r.task().unwrap_or("next step"),
                    r.unit,
                    who.subject
                ),
            ),
        ),
        _ => page::refused(409, v.as_str().unwrap_or("the run was not granted")),
    }
}

// The `MainRun` side: runs in its storage, their jobs on Runners, polled by its alarm.

async fn load(state: &State) -> Result<Runs> {
    Ok(state.storage().get(RUNS).await?.unwrap_or_default())
}

async fn store(state: &State, runs: &Runs) -> Result<()> {
    state.storage().put(RUNS, runs).await
}

/// Each platform unit's newest outputs, for the deploy job.
///
/// # Errors
///
/// When storage fails.
pub async fn kept(state: &State) -> Result<Kept> {
    Ok(state.storage().get(OUTPUTS).await?.unwrap_or_default())
}

/// Arms `state`'s alarm for a poll when none is set.
async fn arm(state: &State) -> Result<()> {
    let storage = state.storage();
    if storage.get_alarm().await?.is_none() {
        storage.set_alarm(Duration::from_millis(0)).await?;
    }
    Ok(())
}

/// `MainRun`'s platform routes, at `path` under `/platform/`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let mut runs = load(state).await?;
    let id = body["id"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or_default();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let t = body["now"].as_u64().unwrap_or_else(now);
    runs.expire(t);
    let answer: std::result::Result<Run, String> = match path {
        "list" => {
            store(state, &runs).await?;
            let shown: Vec<Run> = runs.list.iter().rev().take(20).map(Run::shown).collect();
            return Response::from_json(&json!({ "runs": shown }));
        }
        "propose" => runs.propose(&s("unit"), &s("token"), t),
        "peek" => {
            let at = runs.get(id).map(|r| r.stage).unwrap_or_default();
            runs.peek(id, &s("token"), at, t).map(Run::shown)
        }
        "grant-plan" => runs.grant_plan(id, &s("token"), &s("sealed"), &s("by"), t),
        "grant-apply" => runs.grant_apply(id, &s("token"), &s("by"), t),
        "reoffer" => match runs.list.iter_mut().find(|r| r.id == id) {
            Some(r) if r.stage == Stage::Planned => {
                r.offer = Some(crate::platforms::hash(&s("token")));
                Ok(r.clone())
            }
            _ => Err(format!(
                "platform run {id} has no plan waiting to be applied"
            )),
        },
        "refuse" => runs.refuse(id, &s("by")),
        _ => return Response::error("not found", 404),
    };
    store(state, &runs).await?;
    match answer {
        Ok(run) => {
            if run.task().is_some() {
                arm(state).await?;
            }
            Response::from_json(&run.shown())
        }
        Err(why) => Response::error(why, 409),
    }
}

/// Whether a platform job runs, so `MainRun`'s alarm comes back.
///
/// # Errors
///
/// When storage fails.
pub async fn running(state: &State) -> Result<bool> {
    Ok(!load(state).await?.running().is_empty())
}

/// One poll of `MainRun`'s platform jobs: each one not yet started is started, each finished is
/// recorded, with its next grant offered.
///
/// # Errors
///
/// When storage fails.
pub async fn tick(state: &State, env: &Env, sha: Option<&str>) -> Result<()> {
    let mut runs = load(state).await?;
    let gone = runs.expire(now());
    let jobs: Vec<Run> = runs.running().into_iter().cloned().collect();
    let mut offers: Vec<(Run, &'static str, String)> = Vec::new();
    let mut ended: Vec<u32> = Vec::new();
    for run in jobs {
        let Some(job) = run.job_id() else { continue };
        let report = poll(env, &job).await;
        let attempts = state
            .storage()
            .get::<u32>(&format!("attempts-{job}"))
            .await?
            .unwrap_or_default();
        match report {
            Ok(Some(r)) => {
                let done = finished(&mut runs, &run, &r)?;
                if let Some((next, op, token)) = done {
                    offers.push((next, op, token));
                }
                if let Some(applied) = runs.list.iter().find(|x| x.id == run.id)
                    && applied.stage == Stage::Applied
                    && let Some(o) = r.platform.as_ref().and_then(|v| v["outputs"].as_object())
                {
                    let mut all = kept(state).await?;
                    all.insert(applied.unit.clone(), o.clone());
                    state.storage().put(OUTPUTS, &all).await?;
                }
                ended.push(run.id);
            }
            Ok(None) => {}
            Err(()) if attempts >= ATTEMPTS => {
                let why = format!("its job was lost {ATTEMPTS} times");
                if run.stage == Stage::Planning {
                    let _ = runs.planned(run.id, Err(why), "", now());
                } else {
                    let _ = runs.applied(run.id, Err(why));
                }
                ended.push(run.id);
            }
            Err(()) => {
                state
                    .storage()
                    .put(&format!("attempts-{job}"), attempts + 1)
                    .await?;
                if let Err(why) = start(env, &run, sha).await {
                    worker::console_warn!("platform job {job} did not start: {why}");
                }
            }
        }
    }
    store(state, &runs).await?;
    if let Ok(s) = Settings::from(env) {
        for (run, op, token) in offers {
            let _ = announce(env, &s, &run, op, &token).await;
        }
        for id in ended {
            let Some((kind, said)) = runs
                .list
                .iter()
                .find(|r| r.id == id)
                .and_then(crate::platforms::finish_event)
            else {
                continue;
            };
            let _ = super::events::append(env, &s, kind, None, None, &said, false).await;
        }
        for id in gone {
            let said = format!("platform run {id} expired, its token erased");
            let _ =
                super::events::append(env, &s, "platform.expired", None, None, &said, false).await;
        }
    }
    if !runs.running().is_empty() {
        let storage = state.storage();
        if storage.get_alarm().await?.is_none() {
            storage.set_alarm(POLL).await?;
        }
    }
    Ok(())
}

/// Records `run`'s finished job from `r`: the run, the operation to offer next and its token,
/// when there is one.
fn finished(runs: &mut Runs, run: &Run, r: &Report) -> Result<Option<(Run, &'static str, String)>> {
    let result = |v: &Value| -> std::result::Result<(String, Vec<Value>), String> {
        if r.outcome != Outcome::Passed {
            return Err(r
                .violations
                .first()
                .cloned()
                .unwrap_or_else(|| "the job failed".into()));
        }
        let changes = v["changes"].as_array().cloned().unwrap_or_default();
        Ok((v["plan"].as_str().unwrap_or_default().to_string(), changes))
    };
    let v = r.platform.clone().unwrap_or(Value::Null);
    if let Some(kept) = runs.list.iter_mut().find(|x| x.id == run.id) {
        kept.log = control::tail(&r.log).to_string();
    }
    if run.stage == Stage::Planning {
        let token = platform::random_hex(32)?;
        let next = runs
            .planned(run.id, result(&v), &token, now())
            .map_err(worker::Error::RustError)?;
        return Ok((next.stage == Stage::Planned).then_some((next, "platform.apply", token)));
    }
    runs.applied(run.id, result(&v).map(|(_, c)| c))
        .map_err(worker::Error::RustError)?;
    Ok(None)
}

/// The platform job `id`'s report: `Some` when finished, `Err(())` when it isn't running.
async fn poll(env: &Env, id: &str) -> std::result::Result<Option<Report>, ()> {
    let Ok(stub) = job_stub(env, control::class_of(control::PLATFORM), id) else {
        return Err(());
    };
    let Ok(mut r) = stub
        .fetch_with_str(&format!("https://checker/status/{id}"))
        .await
    else {
        return Ok(None);
    };
    match r.status_code() {
        200 => match r.json::<Report>().await {
            Ok(rep) if rep.outcome == Outcome::Running => Ok(None),
            Ok(rep) => Ok(Some(rep)),
            Err(_) => Ok(None),
        },
        404 => Err(()),
        _ => Ok(None),
    }
}

/// Starts `run`'s job on a Runner from `sha`, with the owner's token unsealed for it alone.
async fn start(env: &Env, run: &Run, sha: Option<&str>) -> std::result::Result<(), String> {
    let job = run.job_id().ok_or("the run has no job to start")?;
    let sha = sha.ok_or("main has no deploy yet to run the job from")?;
    let sealed = run.sealed.as_deref().ok_or("the run's token is gone")?;
    let key = secret(env, crate::cobot::worker::TOKEN_VAR).map_err(|e| e.to_string())?;
    let token = platform::unseal(&key, sealed)
        .await
        .map_err(|e| e.to_string())?;
    let state_key =
        secret(env, crate::control::PLATFORM_STATE_KEY_VAR).map_err(|e| e.to_string())?;
    let (remote, git) = platform::credentials(env, false, crate::cobot::artifacts_wire::TOKEN_TTL)
        .await
        .map_err(|e| e.to_string())?;
    let env_vars = BTreeMap::from([
        ("CLOUDFLARE_API_TOKEN".to_string(), token),
        (
            crate::control::PLATFORM_STATE_KEY_VAR.to_string(),
            state_key,
        ),
    ]);
    let spec = JobSpec {
        id: job.clone(),
        remote,
        token: git,
        base: "main".into(),
        branch: "main".into(),
        sha: sha.to_string(),
        number: 0,
        title: format!("platform run {}", run.id),
        plan: String::new(),
        scope: Vec::new(),
        job: control::PLATFORM.into(),
        argv: run.argv(),
        approver: run.granted_by.last().cloned(),
        since: None,
        env: env_vars,
    };
    let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
    let req = json_request("https://checker/start", Method::Post, Some(&body), None)
        .map_err(|e| e.to_string())?;
    let mut r = job_stub(env, control::class_of(control::PLATFORM), &job)
        .map_err(|e| e.to_string())?
        .fetch_with_request(req)
        .await
        .map_err(|e| e.to_string())?;
    if (200..300).contains(&r.status_code()) {
        Ok(())
    } else {
        Err(r.text().await.unwrap_or_default())
    }
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix("/platform/"),
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
