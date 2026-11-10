//! The `MainRun` Durable Object: `main`'s pipeline. Each push to `main` is recorded; its alarm
//! runs one deploy job at a time, of the newest push, which publishes the sites whose inputs
//! changed since the last deploy (`control::Pipeline`). Beside it, each push is mirrored to
//! GitHub (`mirroring::Mirroring`), and both sides' refs are compared hourly.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::github;
use super::platform::{self, job_stub, json_request, now, secret};
use crate::control::{self, Outcome, Pipeline, Report};
use crate::mirroring::{self, Mirroring};

const PIPELINE: &str = "pipeline";
const MIRRORING: &str = "mirroring";
const STARTING: &str = "starting";

/// How often a running deploy is asked how it went.
const POLL: Duration = Duration::from_secs(20);

/// `main`'s pipeline; its name is `<repo>#main`.
#[durable_object]
pub struct MainRun {
    state: State,
    env: Env,
}

impl MainRun {
    /// Sets an alarm now if a deploy is in flight but none is set: the runtime drops an alarm
    /// that keeps failing, and only an alarm moves a deploy on.
    async fn rearm(&self) -> Result<()> {
        let storage = self.state.storage();
        if self.busy().await? && storage.get_alarm().await?.is_none() {
            storage.set_alarm(Duration::from_millis(0)).await?;
        }
        Ok(())
    }

    /// Whether a deploy is in flight.
    async fn busy(&self) -> Result<bool> {
        Ok(self.pipeline().await?.running.is_some()
            || self
                .state
                .storage()
                .get::<String>(STARTING)
                .await?
                .is_some())
    }

    async fn pipeline(&self) -> Result<Pipeline> {
        Ok(self
            .state
            .storage()
            .get(PIPELINE)
            .await?
            .unwrap_or_default())
    }

    async fn save(&self, p: &Pipeline, starting: Option<&str>) -> Result<()> {
        self.save_after(p, starting, false).await
    }

    /// As [`Self::save`]; with `wait`, a deploy that is starting waits for the next poll, as
    /// when no Container is free.
    async fn save_after(&self, p: &Pipeline, starting: Option<&str>, wait: bool) -> Result<()> {
        let storage = self.state.storage();
        storage.put(PIPELINE, p).await?;
        // `None` would reach the runtime as `undefined`, which `put` refuses.
        match starting {
            Some(sha) => storage.put(STARTING, sha).await?,
            None => {
                storage.delete(STARTING).await?;
            }
        }
        if starting.is_some() && !wait {
            storage.set_alarm(Duration::from_millis(0)).await?;
        } else if starting.is_some() || p.running.is_some() {
            storage.set_alarm(POLL).await?;
        }
        Ok(())
    }

    async fn mirroring(&self) -> Result<Mirroring> {
        Ok(self
            .state
            .storage()
            .get(MIRRORING)
            .await?
            .unwrap_or_default())
    }

    /// Saves `m`, and brings the alarm forward to when the mirror next needs it.
    async fn save_mirroring(&self, m: &Mirroring) -> Result<()> {
        let storage = self.state.storage();
        storage.put(MIRRORING, m).await?;
        let at = now();
        let want = if m.running.is_some() {
            Some(at + POLL.as_secs())
        } else if m.target.is_some() && m.target != m.mirrored {
            Some(m.retry_at.unwrap_or(at).max(at))
        } else {
            None
        };
        if let Some(want) = want {
            let want_ms = i64::try_from(want * 1000).unwrap_or(i64::MAX);
            if storage.get_alarm().await?.is_none_or(|t| t > want_ms) {
                storage
                    .set_alarm(Duration::from_secs(want.saturating_sub(at)))
                    .await?;
            }
        }
        Ok(())
    }

    /// Starts the mirror of `sha` in a job Container, with a read token for Artifacts and an
    /// installation token for GitHub, both minted for this job.
    async fn start_mirror(&self, to: &str, sha: &str) -> std::result::Result<(), String> {
        let (owner, repo) = mirroring::github_repo(to)
            .ok_or_else(|| format!("the mirror `{to}` is not a GitHub repository"))?;
        let to_token =
            github::installation_token(&self.env, &owner, &repo, &mirroring::token_request(&repo))
                .await?;
        let (remote, token) =
            platform::credentials(&self.env, false, cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let spec = control::mirror_spec(&remote, &token, sha, to, &to_token);
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = job_stub(&self.env, control::class_of(control::MIRROR), &spec.id)
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

    /// Moves the mirror on: polls a running job, or starts the newest push's.
    async fn mirror_step(&self) -> Result<()> {
        let Some(to) = github::mirror(&self.env) else {
            return Ok(());
        };
        let mut m = self.mirroring().await?;
        let at = now();
        let start = if let Some((sha, _)) = m.running.clone() {
            let id = control::mirror_id(&sha);
            match self.poll_job(&id, control::MIRROR).await? {
                Ok(Some(report)) => {
                    let ok = report.outcome == Outcome::Passed;
                    let why = report.violations.first().cloned().unwrap_or_default();
                    m.finished(ok, &why, at);
                    if !ok {
                        worker::console_warn!("the mirror of {sha} failed: {why}");
                    }
                    m.start(at)
                }
                Ok(None) => None,
                Err(()) => m.lost(at),
            }
        } else {
            m.start(at)
        };
        if let Some(sha) = start {
            match self.start_mirror(&to, &sha).await {
                Ok(()) => {}
                Err(why) if control::no_capacity(&why) => {
                    // No attempt used: it starts on a later alarm.
                    m.running = None;
                }
                Err(why) => {
                    worker::console_warn!("the mirror of {sha} did not start: {why}");
                    m.finished(false, &why, at);
                }
            }
        }
        self.save_mirroring(&m).await
    }

    /// Compares both sides' refs, when due: what GitHub lacks becomes the mirror's target.
    async fn reconcile(&self) -> Result<Value> {
        let Some(to) = github::mirror(&self.env) else {
            return Ok(json!({ "mirror": null }));
        };
        let mut m = self.mirroring().await?;
        let at = now();
        if !m.reconcile_due(at) {
            return Ok(json!({ "mirror": m }));
        }
        let (remote, token) =
            platform::credentials(&self.env, false, cobot::artifacts_wire::TOKEN_TTL).await?;
        let ours = github::refs(&remote, Some(&token)).await?;
        let theirs = github::refs(&to, None).await?;
        m.reconciled(&ours, &theirs, "main", at);
        self.save_mirroring(&m).await?;
        self.mirror_step().await?;
        Ok(json!({ "mirror": self.mirroring().await? }))
    }

    /// Starts the deploy of `sha` in a job Container, referencing `images`.
    async fn start(
        &self,
        p: &Pipeline,
        sha: &str,
        images: Option<&control::Images>,
    ) -> std::result::Result<(), String> {
        let (remote, token) =
            platform::credentials(&self.env, false, cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let deploy = secret(&self.env, "DEPLOY_TOKEN").map_err(|e| e.to_string())?;
        let mut env = BTreeMap::from([("CLOUDFLARE_API_TOKEN".to_string(), deploy)]);
        // Only the deploy job receives these; see
        // .claude/decisions/2026-11-07-development-control-plane-deploys-itself.md §6.
        for k in cobot::ship_wire::SECRETS {
            if let Ok(v) = secret(&self.env, k) {
                env.insert(k.to_string(), v);
            }
        }
        // Each platform unit's outputs come from its newest apply; see the step 4c amendment of
        // .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md.
        let kept = super::platforms::kept(&self.state)
            .await
            .map_err(|e| e.to_string())?;
        let secret_outputs = env.get(cobot::ship_wire::OUTPUTS_VAR).cloned();
        if let Some(o) = crate::platforms::merged_outputs(secret_outputs.as_deref(), &kept) {
            env.insert(cobot::ship_wire::OUTPUTS_VAR.to_string(), o);
        }
        // The newest images, which a deploy may have built since the secret was set; see
        // .claude/decisions/2026-11-07-development-job-images-on-cloudflare.md §4.
        if let Some(i) = images.and_then(|i| serde_json::to_string(i).ok()) {
            env.insert(cobot::ship_wire::IMAGES_VAR.to_string(), i);
        }
        if let Ok(d) = self.env.var(cobot::ship_wire::DEPLOYMENT_VAR) {
            env.insert(cobot::ship_wire::DEPLOYMENT_VAR.to_string(), d.to_string());
        }
        if let Ok(host) = self.env.var(cobot::ship_wire::HOST_VAR) {
            env.insert(cobot::ship_wire::HOST_VAR.to_string(), host.to_string());
        }
        let spec = control::deploy_spec(&remote, &token, sha, p.deployed.as_deref(), env);
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = job_stub(&self.env, control::class_of(control::DEPLOY), &spec.id)
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

    /// The running deploy's report: `Some` when it finished, `Err(())` when its Container is
    /// gone, `None` while it runs or can't be asked.
    async fn poll(&self, sha: &str) -> Result<std::result::Result<Option<Report>, ()>> {
        self.poll_job(&control::deploy_id(sha), control::DEPLOY)
            .await
    }

    /// Job `id`'s report, for a job of kind `job`, as [`Self::poll`] answers.
    async fn poll_job(
        &self,
        id: &str,
        job: &str,
    ) -> Result<std::result::Result<Option<Report>, ()>> {
        let mut r = job_stub(&self.env, control::class_of(job), id)?
            .fetch_with_str(&format!("https://checker/status/{id}"))
            .await?;
        Ok(match r.status_code() {
            200 => {
                let report: Report = r.json().await?;
                if report.outcome == Outcome::Running {
                    Ok(None)
                } else {
                    let key = format!("ci/{id}.log");
                    self.env
                        .bucket("CI_LOGS")?
                        .put(key, control::tail(&report.log).to_string())
                        .execute()
                        .await?;
                    Ok(Some(report))
                }
            }
            404 => Err(()),
            _ => Ok(None),
        })
    }
}

impl DurableObject for MainRun {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        if path == "/version" {
            return Response::ok(platform::version(&self.env));
        }
        self.rearm().await?;
        if let Some(rest) = path.trim_matches('/').strip_prefix("platform/") {
            let body: Value = req.json().await.unwrap_or(Value::Null);
            return super::platforms::serve(&self.state, rest, &body).await;
        }
        match (req.method(), path.trim_matches('/')) {
            (Method::Post, "pushed") => {
                let body: Value = req.json().await?;
                let Some(sha) = body["sha"].as_str() else {
                    return Response::error("a push names its SHA", 400);
                };
                let mut p = self.pipeline().await?;
                p.pushed(sha);
                let starting = p.start();
                self.save(&p, starting.as_deref()).await?;
                if github::mirror(&self.env).is_some() {
                    let mut m = self.mirroring().await?;
                    m.pushed(sha, now());
                    self.save_mirroring(&m).await?;
                }
                Response::from_json(&p.shown())
            }
            (Method::Post, "retry") => {
                let mut p = self.pipeline().await?;
                let starting = p.retry();
                self.save(&p, starting.as_deref()).await?;
                let mut m = self.mirroring().await?;
                let mirroring = github::mirror(&self.env).is_some()
                    && m.target.is_some()
                    && m.target != m.mirrored
                    && m.running.is_none();
                if mirroring {
                    m.retry_at = None;
                    self.save_mirroring(&m).await?;
                }
                if starting.is_none() && !mirroring {
                    return Response::error(
                        "nothing to retry: a deploy runs or waits, or main has had no deploy",
                        409,
                    );
                }
                Response::from_json(&json!({ "pipeline": p.shown() }))
            }
            (Method::Get, "status") => {
                let p = self.pipeline().await?;
                let m = self.mirroring().await?;
                Response::from_json(&json!({
                    "pipeline": p.shown(),
                    "mirror": m,
                    "mirror_behind": m.behind(now()),
                }))
            }
            (Method::Post, "reconcile") => match self.reconcile().await {
                Ok(v) => Response::from_json(&v),
                Err(e) => Response::error(format!("reconcile: {e}"), 502),
            },
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let mut p = self.pipeline().await?;
        // Platform jobs share the alarm, and never hold up a deploy; see
        // .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 4's amendment.
        if let Err(e) = super::platforms::tick(&self.state, &self.env, p.deployed.as_deref()).await
        {
            worker::console_error!("platform runs: {e}");
        }
        let starting: Option<String> = self.state.storage().get(STARTING).await?;
        let mut next: Option<String> = None;
        let mut wait = false;
        if let Some(sha) = starting {
            let secret = secret(&self.env, cobot::ship_wire::IMAGES_VAR)
                .ok()
                .and_then(|v| serde_json::from_str(&v).ok());
            let images = p.images_for_deploy(secret);
            match self.start(&p, &sha, images.as_ref()).await {
                Ok(()) => {}
                Err(why) if control::no_capacity(&why) => {
                    worker::console_log!("the deploy of {sha} waits for a free Container");
                    next = Some(sha);
                    wait = true;
                }
                Err(why) => {
                    worker::console_warn!("the deploy of {sha} did not start: {why}");
                    next = p.lost(now());
                }
            }
        } else if let Some((sha, _)) = p.running.clone() {
            match self.poll(&sha).await? {
                Ok(Some(report)) => {
                    p.finished(&report, now());
                    deploy_event(&self.env, &sha, &report).await;
                    if report.outcome == Outcome::Failed {
                        let why = format!("the deploy of {sha} failed");
                        let paused = match platform::Settings::from(&self.env) {
                            Ok(s) => super::mandates::pause_all(&self.env, &s, &why).await,
                            Err(e) => Err(e),
                        };
                        if let Err(e) = paused {
                            worker::console_error!("mandates: {e}");
                        }
                    }
                    next = p.start();
                }
                Ok(None) => {}
                Err(()) => next = p.lost(now()),
            }
        } else {
            next = p.start();
        }
        self.save_after(&p, next.as_deref(), wait).await?;
        if let Err(e) = self.mirror_step().await {
            worker::console_error!("mirror: {e}");
        }
        let storage = self.state.storage();
        if super::platforms::running(&self.state).await? {
            // The earliest alarm wins: the mirror's may be an hour away.
            let poll_at = i64::try_from((now() + super::platforms::POLL.as_secs()) * 1000)
                .unwrap_or(i64::MAX);
            if storage.get_alarm().await?.is_none_or(|t| t > poll_at) {
                storage.set_alarm(super::platforms::POLL).await?;
            }
        }
        Response::ok("ok")
    }
}

/// Tells the event log how the deploy of `sha` ended.
async fn deploy_event(env: &Env, sha: &str, report: &Report) {
    let (kind, how) = if report.outcome == Outcome::Passed {
        ("deploy.passed", "passed".to_string())
    } else {
        (
            "deploy.failed",
            format!(
                "failed: {}",
                report.violations.first().cloned().unwrap_or_default()
            ),
        )
    };
    let summary = format!("the deploy of {} {how}", sha.get(..12).unwrap_or(sha));
    let Ok(s) = platform::Settings::from(env) else {
        return;
    };
    if let Err(e) = super::events::append(env, &s, kind, None, None, &summary, false).await {
        worker::console_error!("deploy event: {e}");
    }
    if report.outcome != Outcome::Passed {
        super::push::notify(env, &s, "Deploy failed", &summary, "/approvals").await;
    }
}
