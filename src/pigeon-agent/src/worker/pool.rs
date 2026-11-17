//! Capacity pools in the control plane: the `Pool` Durable Object, one per pool, holding its
//! ledger, and the metering route `POST /v0/pool/<pool>/v1/messages`, which forwards only to
//! the AI Gateway; see `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`
//! §3, §4 and its 2026-11-17 amendment, and `.claude/decisions/2026-11-16-security-ai-gateway-claude-api.md`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use futures_util::StreamExt;
use serde_json::{Value, json};
use worker::{
    DurableObject, Env, Fetch, Headers, Method, Request, RequestInit, Response, Result, State,
    durable_object,
};

use super::Route;
use super::mandates::to_instance;
use super::platform::{self, Settings, json_request, now};
use super::pool_dispatch::{self, Followed};
use crate::access::Identity;
use crate::core::pool::aigateway;
use crate::core::pool::config::{Budget, Pool as PoolConfig, config};
use crate::core::pool::dispatch as pd;
use crate::core::pool::gateway::{Gateway, gateway};
use crate::core::pool::ledger::{self, Ledger};
use crate::core::pool::meter::{self, Seen, Sse, TaskInfo, Vet};
use crate::core::pool::resume;
use crate::core::provider::verdict::{self, ChangeState, Ticket};
use crate::routes::Path;

/// The binding of the `Pool` namespace.
const BINDING: &str = "POOL";

/// The route's prefix.
pub const PREFIX: &str = "/v0/pool/";

/// The AI Gateway's host: the only upstream a pool forwards to.
pub const GATEWAY_BASE: &str = "https://gateway.ai.cloudflare.com";

/// The Worker secret holding the pool's Access service token's client ID.
pub const ACCESS_ID_VAR: &str = "POOL_ACCESS_ID";

/// The Worker secret holding the workspace every response must name.
const WORKSPACE_VAR: &str = "POOL_ANTHROPIC_WORKSPACE";

/// The vars naming the gateway: its account and its name.
const GATEWAY_ACCOUNT_VAR: &str = "AI_GATEWAY_ACCOUNT";
const GATEWAY_VAR: &str = "AI_GATEWAY";

/// The internal header carrying the task the token named, from the route to the object.
const TASK_HEADER: &str = "x-pigeon-pool-task";

/// The grant the Run token is minted from, routed to this object.
const RUN_GRANT: &str = "aigateway";

/// Seconds after which the Run token, living an hour, is minted anew.
const RENEW: u64 = 50 * 60;

/// The storage key of the pool's resume offers.
const RESUME_KEY: &str = "resume-offers";

/// Seconds the Run token lives.
const TTL: u64 = 60 * 60;

/// How long a minted Run token counts as fresh, in seconds: a 401 then is resent once.
const FRESH: u64 = 60;

/// How long to wait before that resend, in seconds.
const FRESH_WAIT: u64 = 30;

/// How often the alarm sweeps while holds or tasks are open, in milliseconds.
const SWEEP_MS: i64 = 60 * 1000;

/// The route's table.
pub(super) const ROUTES: &[Route] = &[Route::only(Method::Post, Path::Prefix(PREFIX), serve_route)];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(serve(req, env, s, who))
}

/// An error in the Messages API's shape, so the harness shows `why` and stops.
fn refused(status: u16, why: &str) -> Result<Response> {
    let body = json!({ "type": "error", "error": { "type": "pool_error", "message": why } });
    Ok(Response::from_json(&body)?.with_status(status))
}

/// The upstream path for the route's tail, or None.
fn upstream_path(tail: &str) -> Option<(&'static str, aigateway::Path)> {
    match tail {
        "v1/messages" => Some(("/v1/messages", aigateway::Path::Messages)),
        "v1/messages/count_tokens" => {
            Some(("/v1/messages/count_tokens", aigateway::Path::CountTokens))
        }
        _ => None,
    }
}

/// The route: checks the task token, then hands the request to its pool's object.
async fn serve(mut req: Request, env: &Env, _s: &Settings, _who: &Identity) -> Result<Response> {
    let path = req.path();
    let rest = &path[PREFIX.len()..];
    let Some((pool, tail)) = rest.split_once('/') else {
        return refused(404, "pool: only /v1/messages and /v1/messages/count_tokens");
    };
    if upstream_path(tail).is_none() {
        return refused(404, "pool: only /v1/messages and /v1/messages/count_tokens");
    }
    let key = meter::task_key(platform::secret(env, "SESSION_KEY")?.as_bytes());
    let token = req.headers().get("x-api-key")?.unwrap_or_default();
    let (named, task) = match meter::verify_task(&key, &token, now()) {
        Ok(v) => v,
        Err(why) => return refused(401, &why),
    };
    if named != pool {
        return refused(401, "pool: the task token is for another pool");
    }
    let body = req.bytes().await?;
    let headers = Headers::new();
    headers.set(TASK_HEADER, &task)?;
    for name in aigateway::KEPT {
        if let Some(v) = req.headers().get(name)? {
            headers.set(name, &v)?;
        }
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(worker::js_sys::Uint8Array::from(&body[..]).into()));
    let inner = Request::new_with_init(&format!("https://pool/forward/{tail}"), &init)?;
    let stub = env
        .durable_object(BINDING)?
        .id_from_name(pool)?
        .get_stub()?;
    platform::call(env, &stub, inner).await
}

/// One pool's ledger, and the requests it meters.
#[durable_object]
pub struct Pool {
    state: State,
    env: Env,
    /// The gateway's Run token and when it was minted; memory only.
    token: RefCell<Option<(String, u64)>>,
    /// Whether a mint is in flight, so only one runs at a time.
    minting: Cell<bool>,
    /// `granter`'s account, when `AI_GATEWAY_ACCOUNT` names none; memory only.
    account: RefCell<Option<String>>,
}

impl DurableObject for Pool {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            token: RefCell::new(None),
            minting: Cell::new(false),
            account: RefCell::new(None),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        if let Some(tail) = path.strip_prefix("/forward/") {
            return self.forward(&mut req, tail).await;
        }
        let body: Value = req.json().await.unwrap_or(Value::Null);
        match path.as_str() {
            "/settle" => self.settle(&body).await,
            "/task/open" => self.open_task(&body).await,
            "/task/close" => self.close_task(&body).await,
            "/task/job" => self.follow_job(&body).await,
            "/status" => self.status().await,
            "/pause" => self.pause(&body).await,
            "/resume/offer" => self.resume_offer(&body).await,
            "/resume/peek" => self.resume_peek(&body).await,
            "/resume/grant" => self.resume_grant(&body).await,
            _ => Response::error("no such pool path", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let Some((pool, _)) = self.pool().await? else {
            return Response::ok("no pool");
        };
        let l = self.ledger(&pool).await?;
        let n = ledger::sweep(&l, &pool, now());
        if n != l {
            self.state.storage().put("ledger", &n).await?;
        }
        if let Err(e) = self.follow(&pool).await {
            worker::console_error!("pool {}: following jobs: {e}", pool.name);
        }
        let n = self.ledger(&pool).await?;
        self.arm(&n).await?;
        Response::ok("swept")
    }
}

impl Pool {
    /// Records the job a task's dispatch started, for the alarm to follow.
    async fn follow_job(&self, body: &Value) -> Result<Response> {
        let (Some(task), Ok(f)) = (
            body["task"].as_str(),
            serde_json::from_value::<Followed>(body["followed"].clone()),
        ) else {
            return Response::error("a job is followed with its task and job", 400);
        };
        let storage = self.state.storage();
        let mut jobs: BTreeMap<String, Followed> = storage.get("jobs").await?.unwrap_or_default();
        jobs.insert(task.to_string(), f);
        storage.put("jobs", &jobs).await?;
        storage.set_alarm(SWEEP_MS).await?;
        Response::ok("followed")
    }

    /// Asks after each followed job; a finished or lost one acts, closes its task and is dropped.
    async fn follow(&self, pool: &PoolConfig) -> Result<()> {
        let storage = self.state.storage();
        let jobs: BTreeMap<String, Followed> = storage.get("jobs").await?.unwrap_or_default();
        if jobs.is_empty() {
            return Ok(());
        }
        let s = Settings::from(&self.env)?;
        let mut left = jobs.clone();
        for (task, f) in &jobs {
            let answer = pool_dispatch::report(&self.env, &f.job).await?;
            let is_gone = matches!(answer, pool_dispatch::Answer::Gone);
            let (first, lost) = pd::gone(f.gone, is_gone, now());
            let report = match answer {
                pool_dispatch::Answer::Ended(r) => Some(*r),
                running if !lost && now() < f.lost_at => {
                    let mut g = f.clone();
                    g.gone = first;
                    // The log so far; see the pool ADR's 2026-11-17 log entry.
                    if let pool_dispatch::Answer::Running(log) = running {
                        g.seen = Some(now());
                        if !log.is_empty() {
                            let end = crate::control::last(&log, pool_dispatch::LIVE_TAIL);
                            g.tail = Some(end.to_string());
                        }
                    }
                    if &g != f {
                        left.insert(task.clone(), g);
                        storage.put("jobs", &left).await?;
                    }
                    continue;
                }
                _ => None,
            };
            let l = self.ledger(pool).await?;
            let requests = l.tasks.get(task).map_or(1, |t| u64::from(t.requests));
            let brief = self.tasks().await?.get(task).map(|i| i.brief.clone());
            // A lost task that spent nothing runs again once; see the pool ADR's 2026-11-17 entry.
            let outcome = match brief {
                Some(b) if report.is_none() && pd::runs_again(&f.label, requests) => {
                    pool_dispatch::again(&self.env, &s, f, &b).await?;
                    "lost".to_string()
                }
                _ => pool_dispatch::act(&self.env, &s, f, report.as_ref()).await?,
            };
            if let Ok(n) = ledger::close_task(&l, task, &outcome, now()) {
                storage.put("ledger", &n).await?;
            }
            left.remove(task);
            storage.put("jobs", &left).await?;
            let said = format!("pool {} task {task} ({}): {outcome}", pool.name, f.label);
            let _ = super::events::append(
                &self.env,
                &s,
                "pool.task.closed",
                Some(f.change),
                Some(f.label.clone()),
                &said,
                false,
            )
            .await;
        }
        if left.len() < jobs.len()
            && let Err(e) = pool_dispatch::reroute(&self.env, &s).await
        {
            worker::console_error!("pool {}: routing waiting briefs: {e}", pool.name);
        }
        Ok(())
    }
}

/// What the object needs about a task's label and change, read before the ledger.
struct Standing {
    ticket: Ticket,
    holds: BTreeSet<u64>,
    change: ChangeState,
}

impl Pool {
    /// The pool's config and gateway, by the name stored at its first task.
    async fn pool(&self) -> Result<Option<(PoolConfig, &'static dyn Gateway)>> {
        let Some(name) = self.state.storage().get::<String>("name").await? else {
            return Ok(None);
        };
        let c = config().map_err(worker::Error::RustError)?;
        let Some(p) = c.pool(&name).cloned() else {
            return Ok(None);
        };
        Ok(gateway(&p.gateway).map(|g| (p, g)))
    }

    async fn ledger(&self, pool: &PoolConfig) -> Result<Ledger> {
        Ok(self
            .state
            .storage()
            .get::<Ledger>("ledger")
            .await?
            .unwrap_or_else(|| ledger::new(pool, now())))
    }

    async fn tasks(&self) -> Result<BTreeMap<String, TaskInfo>> {
        Ok(self.state.storage().get("tasks").await?.unwrap_or_default())
    }

    /// Keeps the alarm set while anything is open.
    async fn arm(&self, l: &Ledger) -> Result<()> {
        let open = !l.holds.is_empty() || l.tasks.values().any(|t| t.closed.is_none());
        if open {
            self.state.storage().set_alarm(SWEEP_MS).await?;
        }
        Ok(())
    }

    /// The task's label's ticket and its change's state, read from the instance.
    async fn standing(&self, info: &TaskInfo) -> Result<Standing> {
        let s = Settings::from(&self.env)?;
        let (_, v) = to_instance(&self.env, &s, "/tickets/list", &json!({})).await?;
        let tickets: crate::tickets::Tickets = serde_json::from_value(v).unwrap_or_default();
        let ticket = match tickets.live(&info.label, now()) {
            Some(_) => Ticket::Live,
            None if tickets.gone_for(&info.label, now()).is_some() => Ticket::Lapsed,
            None => Ticket::Missing,
        };
        let number = u32::try_from(info.change).unwrap_or(u32::MAX);
        let (status, v) =
            super::to_change(&self.env, &s.repo, number, Method::Get, "/status", None).await?;
        let change: Option<crate::core::change::Change> = (status == 200)
            .then(|| serde_json::from_value(v["change"].clone()).ok())
            .flatten();
        let (change, holds) = match change {
            Some(c) => {
                let held = c.session.as_deref() == Some(info.label.as_str());
                (
                    meter::change_state(&c),
                    held.then_some(info.change).into_iter().collect(),
                )
            }
            None => (ChangeState::Closed, BTreeSet::new()),
        };
        Ok(Standing {
            ticket,
            holds,
            change,
        })
    }

    /// The upstream URL and the gateway's Run token, or why there is none.
    async fn upstream(
        &self,
        path: aigateway::Path,
    ) -> std::result::Result<(String, String), &'static str> {
        let var = |n: &str| self.env.var(n).map(|v| v.to_string()).unwrap_or_default();
        let account = self.gateway_account(var(GATEWAY_ACCOUNT_VAR)).await;
        let url = aigateway::url(GATEWAY_BASE, &account, &var(GATEWAY_VAR), path)
            .ok_or("pool: no gateway")?;
        let token = self.gateway_token().await.ok_or("pool: no gateway token")?;
        Ok((url, token))
    }

    /// The gateway's account: `AI_GATEWAY_ACCOUNT` when it is an account ID, else `granter`'s one
    /// account, which the Run token is minted on; see the pool ADR's 2026-11-17 entry on the
    /// gateway's settings.
    async fn gateway_account(&self, from_var: String) -> String {
        if from_var.len() == 32 && from_var.bytes().all(|b| b.is_ascii_hexdigit()) {
            return from_var;
        }
        if let Some(a) = self.account.borrow().clone() {
            return a;
        }
        let found = match super::minting::granter(&self.env).await {
            Ok(g) => platform::account(&g).await.map_err(|e| e.to_string()),
            Err(why) => Err(why),
        };
        match found {
            Ok(a) => {
                *self.account.borrow_mut() = Some(a.clone());
                a
            }
            Err(why) => {
                worker::console_error!("pool: no gateway account: {why}");
                from_var
            }
        }
    }

    /// The gateway's Run token: the one held, under [`RENEW`] old, else a new one from the
    /// `aigateway` grant, minted one at a time; while a mint is in flight, the held token while
    /// it lives, else none. A renewal never revokes the held token, which may be in flight and
    /// ends by its own TTL; only a restart's first mint revokes the registry's previous one; see
    /// the AI Gateway ADR's Run token amendments.
    async fn gateway_token(&self) -> Option<String> {
        let held = self.token.borrow().clone();
        if let Some((t, at)) = &held
            && now() < at + RENEW
        {
            return Some(t.clone());
        }
        if self.minting.get() {
            return held.filter(|(_, at)| now() < at + TTL).map(|(t, _)| t);
        }
        let s = Settings::from(&self.env).ok()?;
        self.minting.set(true);
        let revoke_previous = held.is_none();
        let minted =
            super::minting::for_object(&self.env, &s, RUN_GRANT, "Pool", revoke_previous).await;
        self.minting.set(false);
        match minted {
            Ok(Ok((value, _))) => {
                *self.token.borrow_mut() = Some((value.clone(), now()));
                Some(value)
            }
            Ok(Err(why)) => {
                worker::console_error!("pool: no gateway token: {why}");
                held.filter(|(_, at)| now() < at + TTL).map(|(t, _)| t)
            }
            Err(e) => {
                worker::console_error!("pool: no gateway token: {e}");
                held.filter(|(_, at)| now() < at + TTL).map(|(t, _)| t)
            }
        }
    }

    /// Vets, reserves and forwards one request, then settles it.
    async fn forward(&self, req: &mut Request, tail: &str) -> Result<Response> {
        let Some((route_path, up)) = upstream_path(tail) else {
            return refused(404, "pool: only /v1/messages and /v1/messages/count_tokens");
        };
        let task = req.headers().get(TASK_HEADER)?.unwrap_or_default();
        let body = req.bytes().await?;
        let Some((pool, gw)) = self.pool().await? else {
            return refused(403, "pool: disabled");
        };
        let info = self.tasks().await?.get(&task).cloned();
        let standing = match &info {
            Some(i) => self.standing(i).await?,
            None => Standing {
                ticket: Ticket::Missing,
                holds: BTreeSet::new(),
                change: ChangeState::Closed,
            },
        };
        let label = info.as_ref().map(|i| i.label.clone()).unwrap_or_default();
        let caller = meter::caller(&label, standing.ticket, standing.holds);
        // Nothing mints for a request vet will refuse: a disabled pool, an unknown task, or a
        // ticket or change that fails the broker's check; vet refuses those below, unsent.
        let admitted = pool.enabled
            && info.as_ref().is_some_and(|i| {
                verdict::admit(
                    &caller,
                    meter::CAPABILITY,
                    Some((i.change, standing.change)),
                )
                .is_ok()
            });
        let (url, token) = if admitted {
            match self.upstream(up).await {
                Ok(u) => u,
                Err(why) => return refused(503, why),
            }
        } else {
            (String::new(), String::new())
        };
        let workspace = platform::secret(&self.env, WORKSPACE_VAR).ok();
        // From here to the put, no await but storage: the ledger is read, reserved and written as one.
        let storage = self.state.storage();
        let l = self.ledger(&pool).await?;
        let seq: u64 = storage.get("seq").await?.unwrap_or(0) + 1;
        storage.put("seq", seq).await?;
        let hold = format!("{task}#{seq}");
        let v = Vet {
            gateway: gw,
            pool: &pool,
            ledger: &l,
            workspace: workspace.as_deref(),
            task: &task,
            info: info.as_ref(),
            caller: &caller,
            change: standing.change,
        };
        let forward = match meter::vet(&v, &hold, route_path, &body, now()) {
            Ok(f) => f,
            Err(r) => {
                let closed = r
                    .close
                    .and_then(|o| ledger::close_task(&l, &task, o, now()).ok());
                if let Some(n) = closed {
                    storage.put("ledger", &n).await?;
                }
                return refused(r.status, &r.why);
            }
        };
        storage.put("ledger", &forward.ledger).await?;
        self.arm(&forward.ledger).await?;
        let caller_headers: Vec<(String, String)> = req.headers().entries().collect();
        let Ok(outbound) = aigateway::outbound(&caller_headers, &forward.meta, Some(&token)) else {
            return self.fail(&hold, "pool: bad metadata").await;
        };
        self.send(&url, &outbound, &body, hold, forward.stream, gw, &pool.name)
            .await
    }

    /// Sends a reserved request to the gateway, and settles it: at once, or as its stream ends.
    #[allow(clippy::too_many_arguments)]
    async fn send(
        &self,
        url: &str,
        outbound: &[(String, String)],
        body: &[u8],
        hold: String,
        stream: bool,
        gw: &'static dyn Gateway,
        pool: &str,
    ) -> Result<Response> {
        let attempt = || -> Result<Request> {
            let headers = Headers::new();
            for (n, v) in outbound {
                headers.set(n, v)?;
            }
            let mut init = RequestInit::new();
            init.with_method(Method::Post)
                .with_headers(headers)
                .with_body(Some(worker::js_sys::Uint8Array::from(body).into()));
            Request::new_with_init(url, &init)
        };
        let Ok(mut r) = Fetch::Request(attempt()?).send().await else {
            return self.fail(&hold, "pool: gateway unavailable").await;
        };
        // A Run token minted moments ago may not reach the gateway yet: one resend, later; see
        // the pool ADR's 2026-11-17 entry on a fresh Run token.
        let fresh = self
            .token
            .borrow()
            .as_ref()
            .is_some_and(|(_, at)| now() < at + FRESH);
        if r.status_code() == 401 && fresh {
            worker::Delay::from(std::time::Duration::from_secs(FRESH_WAIT)).await;
            let Ok(again) = Fetch::Request(attempt()?).send().await else {
                return self.fail(&hold, "pool: gateway unavailable").await;
            };
            r = again;
        }
        let status = r.status_code();
        let account = gw
            .account_header()
            .and_then(|h| r.headers().get(h).ok().flatten());
        let ok = (200..300).contains(&status);
        if !(ok && stream) {
            let bytes = r.bytes().await.unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).to_string();
            let usage = if ok { gw.usage_json(&bytes) } else { None };
            self.settle_now(&hold, status, account.as_deref(), &text, usage)
                .await?;
            if status >= 500
                && aigateway::classify(status, &text) != Some(aigateway::Signal::Upstream)
            {
                return refused(502, "pool: gateway unavailable");
            }
            let mut out = Response::from_bytes(bytes)?.with_status(status);
            copy_response_headers(&r, &mut out)?;
            return Ok(out);
        }
        let (env, name) = (self.env.clone(), pool.to_string());
        let mut sse = Sse::default();
        let mut done = false;
        let tee = r.stream()?.map(move |chunk| {
            if let Ok(c) = &chunk {
                sse.feed(gw, c);
            }
            if sse.usage.complete && !done {
                done = true;
                let body = json!({ "hold": hold, "status": status, "account": account, "usage": sse.usage });
                let (env, name) = (env.clone(), name.clone());
                worker::wasm_bindgen_futures::spawn_local(async move {
                    let _ = to_pool(&env, &name, "/settle", &body).await;
                });
            }
            chunk
        });
        let mut out = Response::from_stream(tee)?.with_status(status);
        copy_response_headers(&r, &mut out)?;
        Ok(out)
    }

    /// Charges `hold` its reservation and answers a fail-closed refusal.
    async fn fail(&self, hold: &str, why: &str) -> Result<Response> {
        self.settle_now(hold, 0, None, "", None).await?;
        refused(502, why)
    }

    async fn settle(&self, body: &Value) -> Result<Response> {
        let hold = body["hold"].as_str().unwrap_or_default();
        let status = body["status"]
            .as_u64()
            .and_then(|s| u16::try_from(s).ok())
            .unwrap_or(0);
        let usage = serde_json::from_value(body["usage"].clone()).ok();
        self.settle_now(hold, status, body["account"].as_str(), "", usage)
            .await?;
        Response::ok("settled")
    }

    /// Settles `hold` and files an owner item for each tripwire.
    async fn settle_now(
        &self,
        hold: &str,
        status: u16,
        account: Option<&str>,
        text: &str,
        usage: Option<crate::core::pool::gateway::Usage>,
    ) -> Result<()> {
        let Some((pool, gw)) = self.pool().await? else {
            return Ok(());
        };
        let workspace = platform::secret(&self.env, WORKSPACE_VAR).unwrap_or_default();
        let l = self.ledger(&pool).await?;
        let seen = Seen {
            status,
            account,
            body: text,
            usage,
        };
        let task = l
            .holds
            .get(hold)
            .map(|h| h.task.clone())
            .unwrap_or_default();
        let first = l.tasks.get(&task).is_some_and(|t| t.requests == 1);
        let event = meter::upstream_event(gw, &pool.name, &task, &seen, first);
        let s = meter::settle(gw, &pool, &l, hold, &workspace, &seen, now());
        self.state.storage().put("ledger", &s.ledger).await?;
        if let (Some(e), Ok(settings)) = (event, Settings::from(&self.env)) {
            let said = format!("pool {} upstream: {e}", pool.name);
            let _ = super::events::append(
                &self.env,
                &settings,
                "pool.upstream",
                None,
                None,
                &said,
                false,
            )
            .await;
        }
        for why in &s.trips {
            self.tripped(&pool.name, why).await;
        }
        Ok(())
    }

    /// A tripwire: an event and an owner item, as the pool ADR §2 says.
    async fn tripped(&self, pool: &str, why: &str) {
        let Ok(s) = Settings::from(&self.env) else {
            return;
        };
        let said = format!("pool {pool} paused: {why}");
        let _ = super::events::append(&self.env, &s, "pool.state", None, None, &said, true).await;
        let mut args = serde_json::Map::new();
        args.insert("kind".into(), json!("action"));
        args.insert("title".into(), json!(format!("Pool {pool} paused")));
        args.insert(
            "detail".into(),
            json!(format!(
                "{why}. Resuming it is yours, per .claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md §2 and §8."
            )),
        );
        if let Err(e) = super::owner_items::run(&self.env, &s, "owner.ask", &args).await {
            worker::console_error!("pool trip: {e}");
        }
    }

    /// Opens a task: its info beside the ledger, and its budget held against the admit line.
    async fn open_task(&self, body: &Value) -> Result<Response> {
        let storage = self.state.storage();
        let name = body["pool"].as_str().unwrap_or_default();
        if storage.get::<String>("name").await?.is_none() {
            storage.put("name", name).await?;
        }
        let Some((pool, _)) = self.pool().await? else {
            return Response::error("pool: no such pool", 404);
        };
        let (Ok(info), Ok(budget), Some(task)) = (
            serde_json::from_value::<TaskInfo>(body["info"].clone()),
            serde_json::from_value::<Budget>(body["budget"].clone()),
            body["task"].as_str(),
        ) else {
            return Response::error("a task opens with its task, info and budget", 400);
        };
        let class = match info.class.as_str() {
            "haiku" => crate::sessions::Class::Haiku,
            "sonnet" => crate::sessions::Class::Sonnet,
            _ => return Response::error("pool: only haiku and sonnet tasks", 400),
        };
        let l = self.ledger(&pool).await?;
        match ledger::admit_task(&l, &pool, task, class, budget, now()) {
            Ok(n) => {
                let mut tasks = self.tasks().await?;
                tasks.insert(task.to_string(), info);
                storage.put("tasks", &tasks).await?;
                storage.put("ledger", &n).await?;
                self.arm(&n).await?;
                Response::from_json(&json!({ "task": task, "deadline": n.tasks[task].deadline }))
            }
            Err(r) => Response::error(r.to_string(), 409),
        }
    }

    async fn close_task(&self, body: &Value) -> Result<Response> {
        let Some((pool, _)) = self.pool().await? else {
            return Response::error("pool: no such pool", 404);
        };
        let task = body["task"].as_str().unwrap_or_default();
        let outcome = body["outcome"].as_str().unwrap_or("closed");
        let l = self.ledger(&pool).await?;
        match ledger::close_task(&l, task, outcome, now()) {
            Ok(n) => {
                self.state.storage().put("ledger", &n).await?;
                Response::from_json(&json!({ "task": task, "spent": n.tasks[task].spent }))
            }
            Err(r) => Response::error(r.to_string(), 404),
        }
    }

    async fn status(&self) -> Result<Response> {
        let Some((pool, _)) = self.pool().await? else {
            return Response::from_json(&json!({ "state": "unused" }));
        };
        let l = ledger::roll(&self.ledger(&pool).await?, &pool, now());
        let state = ledger::state(&l, &pool, now());
        let jobs: BTreeMap<String, Followed> =
            self.state.storage().get("jobs").await?.unwrap_or_default();
        Response::from_json(&json!({ "state": state, "ledger": l, "jobs": jobs }))
    }

    /// Stores the object's pool name from `body["pool"]`, when it has none and the pool exists.
    async fn name(&self, body: &Value) -> Result<()> {
        let Some(name) = body["pool"].as_str() else {
            return Ok(());
        };
        let storage = self.state.storage();
        let known = config().is_ok_and(|c| c.pool(name).is_some());
        if known && storage.get::<String>("name").await?.is_none() {
            storage.put("name", name).await?;
        }
        Ok(())
    }

    async fn pause(&self, body: &Value) -> Result<Response> {
        self.name(body).await?;
        let Some((pool, _)) = self.pool().await? else {
            return Response::error("pool: no such pool", 404);
        };
        let why = body["why"].as_str().unwrap_or("paused");
        let n = ledger::pause(&self.ledger(&pool).await?, why);
        self.state.storage().put("ledger", &n).await?;
        Response::ok("paused")
    }

    /// Offers a resume for the owner's passkey; see `core::pool::resume`.
    async fn resume_offer(&self, body: &Value) -> Result<Response> {
        self.name(body).await?;
        let Some((pool, _)) = self.pool().await? else {
            return Response::error("pool: no such pool", 404);
        };
        let (Some(token), Some(expires)) = (body["token"].as_str(), body["expires"].as_u64())
        else {
            return Response::error("an offer has a token and an expiry", 400);
        };
        let storage = self.state.storage();
        let mut all: resume::Offers = storage.get(RESUME_KEY).await?.unwrap_or_default();
        let offer = all.offer(&pool.name, token, now(), expires);
        storage.put(RESUME_KEY, &all).await?;
        Response::from_json(&offer)
    }

    async fn resume_peek(&self, body: &Value) -> Result<Response> {
        let storage = self.state.storage();
        let all: resume::Offers = storage.get(RESUME_KEY).await?.unwrap_or_default();
        match all.peek(body["token"].as_str().unwrap_or_default(), now()) {
            Ok(o) => Response::from_json(o),
            Err(why) => Response::error(why, 403),
        }
    }

    /// Spends the offer and resumes the pool; only the page calls it, after the passkey.
    async fn resume_grant(&self, body: &Value) -> Result<Response> {
        let Some((pool, _)) = self.pool().await? else {
            return Response::error("pool: no such pool", 404);
        };
        let storage = self.state.storage();
        let mut all: resume::Offers = storage.get(RESUME_KEY).await?.unwrap_or_default();
        let taken = all.take(body["token"].as_str().unwrap_or_default(), now());
        storage.put(RESUME_KEY, &all).await?;
        if let Err(why) = taken {
            return Response::error(why, 409);
        }
        let n = ledger::resume(&self.ledger(&pool).await?);
        storage.put("ledger", &n).await?;
        Response::ok("resumed")
    }
}

/// Copies the upstream response's content type and Anthropic headers, never the gateway's own.
fn copy_response_headers(from: &Response, to: &mut Response) -> Result<()> {
    let h = to.headers_mut();
    for (n, v) in from.headers().entries() {
        let n = n.to_ascii_lowercase();
        if n == "content-type"
            || n == "retry-after"
            || n.starts_with("anthropic-")
            || n == "request-id"
        {
            h.set(&n, &v)?;
        }
    }
    Ok(())
}

/// Sends `path` to pool `name`'s object.
pub(super) async fn to_pool(
    env: &Env,
    name: &str,
    path: &str,
    body: &Value,
) -> Result<(u16, Value)> {
    let req = json_request(
        &format!("https://pool{path}"),
        Method::Post,
        Some(body),
        None,
    )?;
    let stub = env
        .durable_object(BINDING)?
        .id_from_name(name)?
        .get_stub()?;
    let mut r = platform::call(env, &stub, req).await?;
    let status = r.status_code();
    let text = r.text().await?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    ))
}
