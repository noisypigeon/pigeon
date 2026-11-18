//! Benches in the control plane: the `Bench` Durable Object, one per bench, holding its
//! Container; the `Instance`'s fleet under the cap; and the `bench.*` operations; see
//! `.claude/decisions/2026-11-16-development-session-bench-containers.md` §1, §3, §4 and §6.

use serde_json::{Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::mandates::to_instance;
use super::platform::{self, Settings, json_request, now};
use crate::core::bench::{
    self as rules,
    admit::{Admit, Fleet, Limits},
};

/// The `Instance`'s storage key for the fleet.
const FLEET: &str = "bench-fleet";

/// The `Instance`'s storage key for the sessions that have had a bench, whose scratch ref
/// `session.end` deletes.
const USED: &str = "bench-used";

/// The first time a day's budget refuses a bench: one owner item, the owner's to answer by
/// raising `bench_hours` or not (§6).
async fn budget_reached(env: &Env, s: &Settings, label: &str) {
    let Ok((_, first)) = to_instance(env, s, "/benches/asked", &json!({})).await else {
        return;
    };
    if first != json!(true) {
        return;
    }
    let mut args = serde_json::Map::new();
    args.insert("kind".into(), json!("action"));
    args.insert("session".into(), json!(label));
    args.insert(
        "title".into(),
        json!(format!(
            "Benches reached today's {} Container-hours",
            rules::DEFAULT_HOURS
        )),
    );
    args.insert(
        "detail".into(),
        json!(
            "New fan-out benches are refused until 00:00 UTC; every session keeps its first bench. Raising `bench_hours` is yours, per .claude/decisions/2026-11-16-development-session-bench-containers.md §6."
        ),
    );
    if let Err(e) = super::owner_items::run(env, s, "owner.ask", &args).await {
        worker::console_error!("bench budget: {e}");
    }
}

/// At `session.end`: stops `label`'s benches and, when it ever had one, deletes its scratch
/// ref through an unscratch job (§4).
///
/// # Errors
///
/// When a binding fails.
pub async fn end_session(env: &Env, s: &Settings, label: &str) -> Result<()> {
    let Some(session) = rules::session_id(label) else {
        return Ok(());
    };
    let (_, v) = to_instance(env, s, "/benches/end", &json!({ "session": session })).await?;
    for name in v["names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let _ = to_bench(env, name, "/stop", &json!({})).await;
    }
    if v["used"] != json!(true) {
        return Ok(());
    }
    let (remote, token) =
        platform::credentials(env, true, crate::wire::artifacts_wire::TOKEN_TTL).await?;
    let spec = crate::control::unscratch_spec(session, &remote, &token);
    let body = serde_json::to_value(&spec)?;
    let req = json_request("https://checker/start", Method::Post, Some(&body), None)?;
    let class = crate::control::class_of(crate::control::UNSCRATCH);
    platform::job_stub(env, class, &spec.id)?
        .fetch_with_request(req)
        .await?;
    Ok(())
}

/// The binding of the `Bench` namespace.
const BINDING: &str = "BENCH";

/// How often a bench's alarm checks its ticket and its idleness, in milliseconds.
const TICK_MS: i64 = 5 * 60 * 1000;

/// The bench's own signing key, derived from the Worker's, so a leak opens only this bench.
fn key(env: &Env, name: &str) -> Result<String> {
    let token = super::checker::signing_key(env)?;
    Ok(crate::wire::link::derive(&token, &format!("bench:{name}")))
}

/// One bench's Container, named `<session_id>.<k>`.
#[durable_object]
pub struct Bench {
    state: State,
    env: Env,
}

impl DurableObject for Bench {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let body: Value = req.json().await.unwrap_or(Value::Null);
        match path.as_str() {
            "/open" => self.open(&body).await,
            "/stop" => self.stop("stopped").await,
            p if p.starts_with("/call/") => self.call(&p["/call/".len()..], &body).await,
            _ => Response::error("no such bench path", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let storage = self.state.storage();
        let label: Option<String> = storage.get("label").await.ok().flatten();
        let name: Option<String> = storage.get("name").await.ok().flatten();
        let (Some(label), Some(name)) = (label, name) else {
            return Response::ok("no bench");
        };
        // The standby only reads; see the blue-green ADR §A.3.
        if !super::lease::acting(&self.env).await {
            storage.set_alarm(TICK_MS).await?;
            return Response::ok("standby");
        }
        let s = Settings::from(&self.env)?;
        if !live(&self.env, &s, &label).await? {
            let _ = release(&self.env, &s, &name).await;
            return self.stop("its session's ticket ended").await;
        }
        let Some(container) = self.state.container() else {
            return Response::ok("no container");
        };
        if !container.running() {
            return Response::ok("asleep");
        }
        self.meter(&s).await;
        let idle: u64 = storage.get("idle").await.ok().flatten().unwrap_or(30);
        let used: u64 = storage.get("used").await.ok().flatten().unwrap_or(0);
        let busy = self.busy().await;
        if !busy && now().saturating_sub(used) >= idle * 60 {
            let _ = release(&self.env, &s, &name).await;
            return self.stop("idle").await;
        }
        storage.set_alarm(TICK_MS).await?;
        Response::ok("kept")
    }
}

impl Bench {
    /// Records the bench's label, name and idle timeout, starts its Container, and arms its alarm.
    async fn open(&self, body: &Value) -> Result<Response> {
        let storage = self.state.storage();
        let text = |k: &str| body[k].as_str().unwrap_or_default().to_string();
        let (label, name) = (text("label"), text("name"));
        if rules::parse_name(&name).is_none() || label.is_empty() {
            return Response::error("a bench opens with its label and name", 400);
        }
        storage.put("label", &label).await?;
        storage.put("name", &name).await?;
        storage
            .put("idle", body["idle"].as_u64().unwrap_or(30))
            .await?;
        storage.put("used", now()).await?;
        if let Err(e) = self.ensure(&name).await {
            return Response::error(e, 502);
        }
        storage.set_alarm(TICK_MS).await?;
        Response::from_json(&json!({ "name": name, "running": true }))
    }

    /// Starts the Container when it isn't running, held for its life.
    async fn ensure(&self, name: &str) -> std::result::Result<(), String> {
        let container = self
            .state
            .container()
            .ok_or("no Container is bound to Bench")?;
        if container.running() {
            return Ok(());
        }
        let launch = crate::containers::worker::JobsLaunch {
            token: key(&self.env, name).map_err(|e| e.to_string())?,
            build: super::latest_build(&self.env).await,
        };
        crate::containers::worker::start_bench(&self.state, &container, &launch)
            .await
            .map_err(|e| e.to_string())?;
        let storage = self.state.storage();
        let _ = storage.put("started", now()).await;
        let _ = storage.put("metered", now()).await;
        if let Some(held) = self.state.container() {
            self.state.wait_until(async move {
                let _ = crate::containers::worker::monitor(&held).await;
            });
        }
        Ok(())
    }

    /// Forwards `route` to the Container's `/v0/bench/<route>`, signed under the bench's key.
    async fn call(&self, route: &str, body: &Value) -> Result<Response> {
        let storage = self.state.storage();
        let Some(name) = storage.get::<String>("name").await.ok().flatten() else {
            return Response::error("this bench isn't open", 409);
        };
        if let Err(e) = self.ensure(&name).await {
            return Response::error(e, 502);
        }
        storage.put("used", now()).await?;
        let Some(container) = self.state.container() else {
            return Response::error("no Container is bound to Bench", 500);
        };
        let bytes = serde_json::to_vec(body).unwrap_or_default();
        let token = key(&self.env, &name)?;
        match crate::containers::worker::call(
            &container,
            &token,
            Method::Post,
            &format!("/v0/bench/{route}"),
            &bytes,
        )
        .await
        {
            Ok((status, v)) => {
                if route == "status" && status == 200 {
                    self.record(&name, &token, &v).await;
                }
                Ok(Response::from_json(&v)?.with_status(status))
            }
            Err(e) => Response::error(e.to_string(), 502),
        }
    }

    /// The first time a run is read as ended: its whole log to `CI_LOGS` at
    /// `bench/<session_id>/<run>.log`, and `bench.finished` for its session.
    async fn record(&self, name: &str, token: &str, v: &Value) {
        let (Some(run), Some(state)) = (v["run"].as_str(), v["state"].as_str()) else {
            return;
        };
        if matches!(state, "syncing" | "running") {
            return;
        }
        let storage = self.state.storage();
        let mark = format!("done:{run}");
        if storage.get::<bool>(&mark).await.ok().flatten().is_some() {
            return;
        }
        let _ = storage.put(&mark, true).await;
        let Some(container) = self.state.container() else {
            return;
        };
        let mut log = String::new();
        let mut offset = 0u64;
        for _ in 0..200 {
            let ask =
                serde_json::to_vec(&json!({ "run": run, "offset": offset })).unwrap_or_default();
            let Ok((200, page)) = crate::containers::worker::call(
                &container,
                token,
                Method::Post,
                "/v0/bench/status",
                &ask,
            )
            .await
            else {
                break;
            };
            let text = page["log"].as_str().unwrap_or_default();
            if text.is_empty() {
                break;
            }
            log.push_str(text);
            offset = page["next"].as_u64().unwrap_or(offset);
        }
        let session = rules::parse_name(name).map_or("", |(id, _)| id);
        let key = format!("bench/{session}/{run}.log");
        if let Ok(bucket) = self.env.bucket("CI_LOGS") {
            let _ = bucket.put(key.clone(), log).execute().await;
        }
        let label: Option<String> = storage.get("label").await.ok().flatten();
        let said = format!(
            "bench {name} run {run} {state} in {}s (exit {}); log {key}",
            v["seconds"].as_u64().unwrap_or(0),
            v["exit"]
                .as_i64()
                .map_or_else(|| "none".to_string(), |e| e.to_string()),
        );
        if let Ok(s) = Settings::from(&self.env) {
            let _ =
                super::events::append(&self.env, &s, "bench.finished", None, label, &said, false)
                    .await;
        }
    }

    /// Adds the Container's seconds since the last meter to the fleet's day.
    async fn meter(&self, s: &Settings) {
        let storage = self.state.storage();
        let started: u64 = storage.get("started").await.ok().flatten().unwrap_or(0);
        let metered: u64 = storage.get("metered").await.ok().flatten().unwrap_or(0);
        let from = started.max(metered);
        if from == 0 {
            return;
        }
        let at = now();
        let seconds = at.saturating_sub(from);
        if to_instance(
            &self.env,
            s,
            "/benches/meter",
            &json!({ "seconds": seconds }),
        )
        .await
        .is_ok()
        {
            let _ = storage.put("metered", at).await;
        }
    }

    /// Whether the Container runs a command now.
    async fn busy(&self) -> bool {
        let Some(name) = self
            .state
            .storage()
            .get::<String>("name")
            .await
            .ok()
            .flatten()
        else {
            return false;
        };
        let (Some(container), Ok(token)) = (self.state.container(), key(&self.env, &name)) else {
            return false;
        };
        matches!(
            crate::containers::worker::call(&container, &token, Method::Post, "/v0/bench/info", b"{}").await,
            Ok((200, v)) if !v["running"].is_null()
        )
    }

    /// Destroys the Container and forgets the bench.
    async fn stop(&self, why: &str) -> Result<Response> {
        if let Some(container) = self.state.container()
            && container.running()
        {
            if let Ok(s) = Settings::from(&self.env) {
                self.meter(&s).await;
            }
            let _ = container.destroy(Some(why)).await;
        }
        let storage = self.state.storage();
        storage.delete_all().await?;
        let _ = storage.delete_alarm().await;
        Response::from_json(&json!({ "stopped": true, "why": why }))
    }
}

/// Whether `label` holds a live ticket.
async fn live(env: &Env, s: &Settings, label: &str) -> Result<bool> {
    let (_, v) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(v).unwrap_or_default();
    Ok(tickets.live(label, now()).is_some())
}

/// Drops bench `name` from the fleet.
async fn release(env: &Env, s: &Settings, name: &str) -> Result<()> {
    to_instance(env, s, "/benches/release", &json!({ "name": name })).await?;
    Ok(())
}

/// Sends `path` to bench `name`'s object.
async fn to_bench(env: &Env, name: &str, path: &str, body: &Value) -> Result<(u16, Value)> {
    let req = json_request(
        &format!("https://bench{path}"),
        Method::Post,
        Some(body),
        None,
    )?;
    let mut r = env
        .durable_object(BINDING)?
        .id_from_name(name)?
        .get_stub()?
        .fetch_with_request(req)
        .await?;
    let status = r.status_code();
    let v = r
        .json::<Value>()
        .await
        .unwrap_or_else(|_| json!({ "error": "the bench answered no JSON" }));
    Ok((status, v))
}

/// The `Instance`'s fleet: `/benches/admit`, `/touch`, `/release`, `/end` and `/list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let fleet: Fleet = storage.get(FLEET).await.ok().flatten().unwrap_or_default();
    let text = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let at = body["now"].as_u64().unwrap_or_else(now);
    let (next, answer) = match path {
        "/benches/admit" => {
            let k = body["k"]
                .as_u64()
                .and_then(|k| u8::try_from(k).ok())
                .unwrap_or(u8::MAX);
            let (f, a) = crate::core::bench::admit::admit(
                &fleet,
                Limits::default(),
                &text("session"),
                k,
                at,
            );
            if matches!(a, Admit::Have | Admit::Start | Admit::Reclaim { .. }) {
                let mut used: std::collections::BTreeSet<String> =
                    storage.get(USED).await.ok().flatten().unwrap_or_default();
                if used.insert(text("session")) {
                    storage.put(USED, &used).await?;
                }
            }
            (Some(f), json!(a))
        }
        "/benches/meter" => (
            Some(crate::core::bench::admit::meter(
                &fleet,
                body["seconds"].as_u64().unwrap_or(0),
                at,
            )),
            json!({ "ok": true }),
        ),
        "/benches/asked" => {
            let first = !fleet.asked;
            (
                Some(crate::core::bench::admit::mark_asked(&fleet)),
                json!(first),
            )
        }
        "/benches/touch" => (
            Some(crate::core::bench::admit::touch(
                &fleet,
                &text("name"),
                body["busy"].as_bool().unwrap_or(false),
                at,
            )),
            json!({ "ok": true }),
        ),
        "/benches/release" => (
            Some(crate::core::bench::admit::release(&fleet, &text("name"))),
            json!({ "ok": true }),
        ),
        "/benches/end" => {
            let (f, names) = crate::core::bench::admit::end_session(&fleet, &text("session"));
            let mut used: std::collections::BTreeSet<String> =
                storage.get(USED).await.ok().flatten().unwrap_or_default();
            let was = used.remove(&text("session"));
            if was {
                storage.put(USED, &used).await?;
            }
            (Some(f), json!({ "names": names, "used": was }))
        }
        "/benches/list" => (None, json!(fleet)),
        _ => return Response::error("no such fleet path", 404),
    };
    if let Some(f) = next {
        storage.put(FLEET, &f).await?;
    }
    Response::from_json(&answer)
}

/// Argument `k` as a number, given as JSON or as text.
fn number(args: &serde_json::Map<String, Value>, k: &str) -> Option<u64> {
    match args.get(k)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Argument `k` as a flag, given as JSON or as text.
fn flag(args: &serde_json::Map<String, Value>, k: &str) -> bool {
    match args.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim() == "true",
        _ => false,
    }
}

/// The session ID of `ticket`'s live label, or why not.
async fn session_of(
    env: &Env,
    s: &Settings,
    args: &serde_json::Map<String, Value>,
) -> Result<std::result::Result<(String, String), (u16, Value)>> {
    let ticket = args.get("ticket").and_then(Value::as_str);
    let label = match super::tickets::holder(env, s, ticket).await? {
        Ok(label) => label,
        Err(why) => return Ok(Err((403, json!(why)))),
    };
    match rules::session_id(&label) {
        Some(id) => Ok(Ok((label.clone(), id.to_string()))),
        None => Ok(Err((
            400,
            json!(format!(
                "bench: {label} isn't a Claude Code session's label"
            )),
        ))),
    }
}

/// Admits and opens bench `k` of `session`, stopping a reclaimed one: its name, or why not.
async fn admitted(
    env: &Env,
    s: &Settings,
    label: &str,
    session: &str,
    k: u8,
) -> Result<std::result::Result<String, (u16, Value)>> {
    let (_, a) = to_instance(
        env,
        s,
        "/benches/admit",
        &json!({ "session": session, "k": k }),
    )
    .await?;
    let admit: Admit = match serde_json::from_value(a) {
        Ok(a) => a,
        Err(e) => {
            return Ok(Err((
                500,
                json!(format!("bench: the fleet answered badly: {e}")),
            )));
        }
    };
    match admit {
        Admit::Have | Admit::Start => {}
        Admit::Reclaim { victim } => {
            let _ = to_bench(env, &victim, "/stop", &json!({})).await;
        }
        Admit::Wait { ahead } => {
            return Ok(Err((
                409,
                json!(format!("wait: bench capacity, {ahead} ahead")),
            )));
        }
        Admit::Refused { why } => {
            if why.starts_with("wait: bench budget") {
                budget_reached(env, s, label).await;
            }
            return Ok(Err((409, json!(why))));
        }
    }
    let name = rules::name(session, k).unwrap_or_default();
    let open = json!({ "label": label, "name": name, "idle": rules::DEFAULT_IDLE_MINUTES });
    let (status, v) = to_bench(env, &name, "/open", &open).await?;
    if status != 200 {
        let _ = release(env, s, &name).await;
        return Ok(Err((502, v)));
    }
    Ok(Ok(name))
}

/// `bench.run`: an allowed command at `sha` on bench `k`, admitted under the cap.
async fn run(
    env: &Env,
    s: &Settings,
    label: &str,
    session: &str,
    k: u8,
    args: &serde_json::Map<String, Value>,
) -> Result<(u16, Value)> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let asked: Vec<String> = args
        .get("argv")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let command = match rules::allow::allow(&asked) {
        Ok(c) => c,
        Err(r) => return Ok((400, json!(r.to_string()))),
    };
    let Some(sha) =
        text("sha").filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return Ok((
            400,
            json!("bench: `sha` is the full SHA `bench push` printed"),
        ));
    };
    let timeout = number(args, "timeout").and_then(|t| u32::try_from(t).ok());
    if let Err(e) = rules::timeout(timeout) {
        return Ok((400, json!(e)));
    }
    let name = match admitted(env, s, label, session, k).await? {
        Ok(n) => n,
        Err(refused) => return Ok(refused),
    };
    let (remote, token) =
        platform::credentials(env, false, crate::wire::artifacts_wire::TOKEN_TTL).await?;
    let run = format!("{k}-{}", platform::random_hex(6)?);
    let spec = json!({
        "run": run,
        "remote": remote,
        "token": token,
        "ref": rules::scratch_ref(session),
        "sha": sha,
        "base": "main",
        "command": command,
        "timeout": timeout,
    });
    let answer = to_bench(env, &name, "/call/run", &spec).await?;
    if answer.0 == 202 {
        let busy = json!({ "name": name, "busy": true });
        let _ = to_instance(env, s, "/benches/touch", &busy).await;
    }
    Ok(answer)
}

/// `bench.run`, `bench.status`, `bench.logs`, `bench.cancel`, `bench.reset`, `bench.list` and
/// `bench.warm`, for the holder of a live ticket, on its own benches only.
///
/// # Errors
///
/// When a binding fails.
pub async fn operation(
    env: &Env,
    s: &Settings,
    op: &str,
    args: &serde_json::Map<String, Value>,
) -> Result<(u16, Value)> {
    let (label, session) = match session_of(env, s, args).await? {
        Ok(x) => x,
        Err(refused) => return Ok(refused),
    };
    let k = match (args.contains_key("bench"), number(args, "bench")) {
        (false, _) => 0,
        (true, Some(k)) if k < u64::from(rules::MAX_PER_SESSION) => u8::try_from(k).unwrap_or(0),
        (true, _) => return Ok((400, json!("bench: `bench` is 0, 1 or 2"))),
    };
    let name = rules::name(&session, k).unwrap_or_default();
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    match op {
        "bench.run" => run(env, s, &label, &session, k, args).await,
        "bench.status" | "bench.logs" => {
            let Some(run) = text("run") else {
                return Ok((400, json!("bench: `run` names the run")));
            };
            let body = json!({
                "run": run,
                "offset": number(args, "offset").unwrap_or(0),
                "wait": number(args, "wait").unwrap_or(0).min(u64::from(rules::MAX_WAIT_SECONDS)),
            });
            let answer = to_bench(env, &name, "/call/status", &body).await?;
            let done = answer.1["state"]
                .as_str()
                .is_some_and(|st| !matches!(st, "syncing" | "running"));
            if done {
                let _ = to_instance(
                    env,
                    s,
                    "/benches/touch",
                    &json!({ "name": name, "busy": false }),
                )
                .await;
            }
            Ok(answer)
        }
        "bench.cancel" => {
            let Some(run) = text("run") else {
                return Ok((400, json!("bench: `run` names the run")));
            };
            to_bench(env, &name, "/call/cancel", &json!({ "run": run })).await
        }
        "bench.reset" => {
            if flag(args, "hard") {
                release(env, s, &name).await?;
                to_bench(env, &name, "/stop", &json!({})).await
            } else {
                to_bench(env, &name, "/call/reset", &json!({})).await
            }
        }
        "bench.warm" => match admitted(env, s, &label, &session, k).await? {
            Ok(name) => to_bench(env, &name, "/call/info", &json!({})).await,
            Err(refused) => Ok(refused),
        },
        "bench.list" => {
            let (_, v) = to_instance(env, s, "/benches/list", &json!({})).await?;
            let fleet: Fleet = serde_json::from_value(v).unwrap_or_default();
            let mine: Vec<Value> = fleet
                .slots
                .iter()
                .filter(|(_, slot)| slot.session == session)
                .map(|(n, slot)| json!({ "name": n, "k": slot.k, "busy": slot.busy, "since": slot.since, "used": slot.used }))
                .collect();
            let waiting = fleet.queue.iter().position(|q| *q == session);
            Ok((200, json!({ "benches": mine, "waiting": waiting })))
        }
        _ => Ok((404, json!(format!("no operation {op}")))),
    }
}
