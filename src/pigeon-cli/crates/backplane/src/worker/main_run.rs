//! The `MainRun` Durable Object: `main`'s pipeline. Each push to `main` is recorded; its alarm
//! runs one deploy job at a time, of the newest push, which publishes the sites whose inputs
//! changed since the last deploy (`control::Pipeline`). Beside it, each push is mirrored to
//! GitHub (`mirroring::Mirroring`), and both sides' refs are compared hourly.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::Digest;
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::github;
use super::platform::{self, job_stub, json_request, now, secret};
use crate::actions::Due;
use crate::control::{self, Outcome, Pipeline, Report};
use crate::iosrelease::{self, IosReleases, Moved};
use crate::mirroring::{self, Mirroring};
use crate::testmaps::{self, TestMaps};

const PIPELINE: &str = "pipeline";
const MIRRORING: &str = "mirroring";
const STARTING: &str = "starting";
const TEST_MAPS: &str = "test_maps";
const IOS_RELEASES: &str = "ios_releases";
const EVICTIONS: &str = "evictions";

/// How long an eviction job may run before it counts as lost.
const EVICTION_DEADLINE: u64 = 30 * 60;

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
            platform::credentials(&self.env, false, crate::cobot::artifacts_wire::TOKEN_TTL)
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
                    if ok {
                        let t = self.test_maps().await?.mirrored(&sha);
                        self.state.storage().put(TEST_MAPS, &t).await?;
                    } else {
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

    async fn ios_releases(&self) -> Result<IosReleases> {
        Ok(self
            .state
            .storage()
            .get(IOS_RELEASES)
            .await?
            .unwrap_or_default())
    }

    /// `ios.release`: starts a release of `main`'s mirrored head; see the 2026-11-12 amendment
    /// to .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md.
    async fn ios_release(&self) -> std::result::Result<Value, String> {
        let m = self.mirroring().await.map_err(|e| e.to_string())?;
        let sha = iosrelease::release_sha(m.target.as_deref(), m.mirrored.as_deref())?;
        let mut r = self.ios_releases().await.map_err(|e| e.to_string())?;
        if let Some(c) = &r.current {
            return Err(format!("release {} is still in flight", c.build));
        }
        let rows = platform::d1_rows(&self.env, &iosrelease::next_build())
            .await
            .map_err(|e| e.to_string())?;
        let build = rows.first().and_then(|v| v["build"].as_u64()).unwrap_or(1);
        let at = now();
        r.start(build, &sha, at)?;
        platform::d1(&self.env, &[iosrelease::insert(build, &sha, at)])
            .await
            .map_err(|e| e.to_string())?;
        self.state
            .storage()
            .put(IOS_RELEASES, &r)
            .await
            .map_err(|e| e.to_string())?;
        self.state
            .storage()
            .set_alarm(Duration::from_secs(1))
            .await
            .map_err(|e| e.to_string())?;
        self.ios_event(
            "ios.release",
            &format!("iOS build {build} of {} started", short(&sha)),
        )
        .await;
        Ok(json!({ "build": build, "sha": sha, "state": "archiving" }))
    }

    /// `ios.status`: the release in flight and the newest releases.
    async fn ios_status(&self) -> Result<Value> {
        let r = self.ios_releases().await?;
        let rows = platform::d1_rows(&self.env, &iosrelease::recent()).await?;
        Ok(json!({ "in_flight": r.view(), "releases": rows }))
    }

    async fn ios_event(&self, kind: &str, summary: &str) {
        let Ok(s) = platform::Settings::from(&self.env) else {
            return;
        };
        if let Err(e) = super::events::append(&self.env, &s, kind, None, None, summary, false).await
        {
            worker::console_error!("ios event: {e}");
        }
    }

    /// Records where the release in flight moved.
    async fn ios_moved(&self, build: u64, moved: &Moved, buildkite: Option<u64>) -> Result<()> {
        let at = now();
        match moved {
            Moved::To(state) => {
                platform::d1(
                    &self.env,
                    &[iosrelease::set_state(build, state, buildkite, at)],
                )
                .await?;
                self.ios_event("ios.release", &format!("iOS build {build}: {state}"))
                    .await;
            }
            Moved::Failed(why) => {
                platform::d1(&self.env, &[crate::apple::fail(build, why, at)]).await?;
                self.ios_event("ios.failed", &format!("iOS build {build} failed: {why}"))
                    .await;
                if let Ok(s) = platform::Settings::from(&self.env) {
                    super::push::notify(&self.env, &s, "iOS release failed", why, "/approvals")
                        .await;
                }
            }
        }
        Ok(())
    }

    /// Moves the release in flight on: GitHub's archive, its copy and Buildkite build, Buildkite's
    /// signing, and App Store Connect's processing.
    async fn ios_step(&self) -> Result<()> {
        let mut r = self.ios_releases().await?;
        let Some(c) = r.current.clone() else {
            return Ok(());
        };
        let at = now();
        match c.stage {
            iosrelease::Stage::Archiving => self.ios_archive_step(&mut r, at).await?,
            iosrelease::Stage::Copying => {
                let moved = match self.ios_copy(&c).await {
                    Ok(number) => {
                        let m = r.signing(number, at);
                        self.ios_moved(c.build, &m, Some(number)).await?;
                        None
                    }
                    Err(why) => Some(r.fail(&why)),
                };
                if let Some(m) = moved {
                    self.ios_moved(c.build, &m, None).await?;
                }
            }
            iosrelease::Stage::Signing => {
                let number = c.buildkite.unwrap_or_default();
                match super::buildkite::state(&self.env, number).await {
                    Ok(state) => {
                        let rows =
                            platform::d1_rows(&self.env, &iosrelease::state_of(c.build)).await?;
                        let row = rows.first().and_then(|v| v["state"].as_str()).unwrap_or("");
                        if let Some(m) = r.signed(&state, row, at) {
                            self.ios_moved(c.build, &m, None).await?;
                        }
                    }
                    Err(why) => worker::console_warn!("iOS build {}: {why}", c.build),
                }
            }
            iosrelease::Stage::Processing => {
                match super::apple::processing(&self.env, c.build).await {
                    Ok(state) => {
                        if let Some(m) = r.processed(state.as_deref()) {
                            self.ios_moved(c.build, &m, None).await?;
                        }
                    }
                    Err(why) => worker::console_warn!("iOS build {}: {why}", c.build),
                }
            }
        }
        self.state.storage().put(IOS_RELEASES, &r).await?;
        if r.busy() {
            let poll_at = i64::try_from((now() + crate::actions::POLL) * 1000).unwrap_or(i64::MAX);
            let storage = self.state.storage();
            if storage.get_alarm().await?.is_none_or(|a| a > poll_at) {
                storage
                    .set_alarm(Duration::from_secs(crate::actions::POLL))
                    .await?;
            }
        }
        Ok(())
    }

    /// Dispatches, finds and polls the release's `ios.archive` run.
    async fn ios_archive_step(&self, r: &mut IosReleases, at: u64) -> Result<()> {
        let Some(c) = r.current.clone() else {
            return Ok(());
        };
        let due = r.actions.due(crate::actions::Mirror::Passed, at);
        let Some((owner, repo)) =
            github::mirror(&self.env).and_then(|to| mirroring::github_repo(&to))
        else {
            return Ok(());
        };
        let request = mirroring::actions_token_request(&repo);
        let token = match github::installation_token(&self.env, &owner, &repo, &request).await {
            Ok(token) => token,
            Err(why) => {
                worker::console_warn!("iOS build {} waits: {why}", c.build);
                return Ok(());
            }
        };
        let check = iosrelease::check(c.build);
        let mut ended: Option<Moved> = None;
        for d in due {
            let call = match d {
                Due::Dispatch(i) => {
                    let body = crate::actions::dispatch_body(
                        &r.actions.dispatches[i],
                        iosrelease::number(c.build),
                        std::slice::from_ref(&check),
                    );
                    github::dispatch(&token, &owner, &repo, &body)
                        .await
                        .map(|()| drop(r.actions.dispatched(i, at)))
                }
                Due::Find(i) => github::runs(&token, &owner, &repo)
                    .await
                    .map(|listed| r.actions.found(i, &listed, at)),
                Due::Poll(i) => {
                    let run = r.actions.dispatches[i].run.unwrap_or_default();
                    github::run(&token, &owner, &repo, run)
                        .await
                        .map(|(run, jobs)| {
                            for f in r.actions.polled(i, &run, &jobs, at) {
                                if f.name == iosrelease::CHECK {
                                    ended = Some(r.archived(i, &f, at));
                                }
                            }
                        })
                }
                Due::Mirror => Ok(()),
            };
            if let (Err(why), Due::Dispatch(i) | Due::Find(i) | Due::Poll(i)) = (call, d) {
                r.actions.failed(i, &why, at);
            }
        }
        if let Some(Moved::Failed(why)) = &ended {
            self.ios_moved(c.build, &Moved::Failed(why.clone()), None)
                .await?;
        }
        Ok(())
    }

    /// Copies the release's archive from its GitHub run to R2, and creates its Buildkite build:
    /// that build's number.
    async fn ios_copy(&self, c: &iosrelease::Current) -> std::result::Result<u64, String> {
        let run = c.run.ok_or("the archive's run is unknown")?;
        let (owner, repo) = github::mirror(&self.env)
            .and_then(|to| mirroring::github_repo(&to))
            .ok_or("no GitHub mirror")?;
        let request = mirroring::actions_token_request(&repo);
        let token = github::installation_token(&self.env, &owner, &repo, &request).await?;
        let body = github::artifact(&token, &owner, &repo, run, iosrelease::ARTIFACT).await?;
        if let Some(why) = iosrelease::invalid(&body) {
            return Err(why);
        }
        self.env
            .bucket("CI_LOGS")
            .map_err(|e| e.to_string())?
            .put(iosrelease::key(&c.sha, c.build), body)
            .execute()
            .await
            .map_err(|e| e.to_string())?;
        super::buildkite::create(&self.env, c.build, &c.sha).await
    }

    /// Opens a `test-map-failed` task on the change whose squash `sha` is, for its session.
    async fn test_map_failed(&self, token: &str, owner: &str, repo: &str, sha: &str) {
        let made = async {
            let message = github::commit_message(token, owner, repo, sha).await?;
            let n = testmaps::merged_change(&message)
                .ok_or_else(|| format!("{sha} names no change"))?;
            let s = platform::Settings::from(&self.env).map_err(|e| e.to_string())?;
            let (_, v) = super::to_change(&self.env, &s.repo, n, Method::Get, "/status", None)
                .await
                .map_err(|e| e.to_string())?;
            let session = v["change"]["session"].as_str().map(str::to_string);
            let note = format!(
                "the test map of main at {} failed after !{n} merged; fix main and push",
                short(sha)
            );
            super::tasks::add(&self.env, &s, "test-map-failed", n, None, session, &note)
                .await
                .map_err(|e| e.to_string())
        };
        if let Err(why) = made.await {
            worker::console_warn!("the test map of {sha} failed, and no task was opened: {why}");
        }
    }

    async fn test_maps(&self) -> Result<TestMaps> {
        Ok(self
            .state
            .storage()
            .get(TEST_MAPS)
            .await?
            .unwrap_or_default())
    }

    /// Moves the test map on: dispatches, finds and polls the `test map` check of `main`'s
    /// newest mirrored commit, and copies a passed one's map to `builds/<sha>/tests.json`.
    async fn test_map_step(&self) -> Result<()> {
        let mut t = self.test_maps().await?;
        let at = now();
        if t.actions.due(crate::actions::Mirror::Passed, at).is_empty()
            && t.copying.is_none()
            && t.deciding.is_none()
            && t.failed_sha.is_none()
        {
            return Ok(());
        }
        let Some((owner, repo)) =
            github::mirror(&self.env).and_then(|to| mirroring::github_repo(&to))
        else {
            return Ok(());
        };
        let request = mirroring::actions_token_request(&repo);
        let token = match github::installation_token(&self.env, &owner, &repo, &request).await {
            Ok(token) => token,
            Err(why) => {
                worker::console_warn!("the test map waits: {why}");
                return Ok(());
            }
        };
        decide(&mut t, &token, &owner, &repo).await;
        let due = t.actions.due(crate::actions::Mirror::Passed, at);
        for d in due {
            let call = match d {
                Due::Dispatch(i) => {
                    let body = crate::actions::dispatch_body(
                        &t.actions.dispatches[i],
                        testmaps::MAIN,
                        &[testmaps::check()],
                    );
                    github::dispatch(&token, &owner, &repo, &body)
                        .await
                        .map(|()| drop(t.actions.dispatched(i, at)))
                }
                Due::Find(i) => github::runs(&token, &owner, &repo)
                    .await
                    .map(|listed| t.actions.found(i, &listed, at)),
                Due::Poll(i) => {
                    let run = t.actions.dispatches[i].run.unwrap_or_default();
                    github::run(&token, &owner, &repo, run)
                        .await
                        .map(|(r, jobs)| {
                            for f in t.actions.polled(i, &r, &jobs, at) {
                                t = t.finished(i, &f);
                            }
                        })
                }
                Due::Mirror => Ok(()),
            };
            if let (Err(why), Due::Dispatch(i) | Due::Find(i) | Due::Poll(i)) = (call, d) {
                t.actions.failed(i, &why, at);
            }
        }
        t = t.prune();
        if let Some(sha) = t.failed_sha.take() {
            self.test_map_failed(&token, &owner, &repo, &sha).await;
        }
        if let Some((sha, run)) = t.copying.clone() {
            let got = github::artifact(&token, &owner, &repo, run, testmaps::ARTIFACT).await;
            match got.and_then(|b| testmaps::invalid(&b, &sha).map_or(Ok(b), Err)) {
                Ok(body) => {
                    let sum = crate::grants::hex(&sha2::Sha256::digest(&body));
                    self.env
                        .bucket("CI_LOGS")?
                        .put(testmaps::key(&sha), body)
                        .execute()
                        .await?;
                    t = t.copied(&sha, &sum);
                }
                Err(why) => {
                    worker::console_warn!("the test map of {sha}: {why}");
                    t = t.copy_failed(&sha, &why);
                }
            }
        }
        self.state.storage().put(TEST_MAPS, &t).await?;
        if t.busy() {
            let poll_at = i64::try_from((now() + crate::actions::POLL) * 1000).unwrap_or(i64::MAX);
            let storage = self.state.storage();
            if storage.get_alarm().await?.is_none_or(|a| a > poll_at) {
                storage
                    .set_alarm(Duration::from_secs(crate::actions::POLL))
                    .await?;
            }
        }
        Ok(())
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
            platform::credentials(&self.env, false, crate::cobot::artifacts_wire::TOKEN_TTL)
                .await?;
        let ours = github::refs(&remote, Some(&token)).await?;
        let theirs = github::refs(&to, None).await?;
        m.reconciled(&ours, &theirs, "main", at);
        self.save_mirroring(&m).await?;
        self.mirror_step().await?;
        Ok(json!({ "mirror": self.mirroring().await? }))
    }

    async fn evictions(&self) -> Result<control::Evictions> {
        Ok(self
            .state
            .storage()
            .get(EVICTIONS)
            .await?
            .unwrap_or_default())
    }

    /// Starts the eviction job on `main`'s deployed head with `candidates`, unless one runs; see
    /// `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §5.
    async fn evictions_run(&self, candidates: &Value) -> std::result::Result<Value, String> {
        let mut e = self.evictions().await.map_err(|e| e.to_string())?;
        if let Some((id, _)) = &e.running {
            return Ok(json!({ "running": id }));
        }
        let p = self.pipeline().await.map_err(|e| e.to_string())?;
        let Some(sha) = p.deployed.clone() else {
            return Err("main has no deploy yet".into());
        };
        let (remote, token) =
            platform::credentials(&self.env, false, crate::cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let spec = control::eviction_spec(&remote, &token, &sha, candidates, now() / 3600);
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = job_stub(&self.env, control::class_of(control::EVICTION), &spec.id)
            .map_err(|e| e.to_string())?
            .fetch_with_request(req)
            .await
            .map_err(|e| e.to_string())?;
        if !(200..300).contains(&r.status_code()) {
            return Err(r.text().await.unwrap_or_default());
        }
        e.running = Some((spec.id.clone(), now()));
        let storage = self.state.storage();
        storage
            .put(EVICTIONS, &e)
            .await
            .map_err(|e| e.to_string())?;
        if storage
            .get_alarm()
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            storage.set_alarm(POLL).await.map_err(|e| e.to_string())?;
        }
        Ok(json!({ "started": spec.id }))
    }

    /// Polls the running eviction job; when it ends, keeps its verdicts and says so in the log.
    async fn eviction_step(&self) -> Result<bool> {
        let mut e = self.evictions().await?;
        let Some((id, started)) = e.running.clone() else {
            return Ok(false);
        };
        let ended = |passed: bool, verdicts: Vec<Value>| control::Judged {
            at: now(),
            id: id.clone(),
            passed,
            verdicts,
        };
        let judged = match self.poll_job(&id, control::EVICTION).await? {
            Ok(Some(report)) => Some(ended(report.outcome == Outcome::Passed, report.evictions)),
            Ok(None) if now().saturating_sub(started) > EVICTION_DEADLINE => {
                Some(ended(false, Vec::new()))
            }
            Ok(None) => None,
            Err(()) => Some(ended(false, Vec::new())),
        };
        let Some(j) = judged else {
            return Ok(true);
        };
        let summary = if j.passed {
            let granted: Vec<String> = j.granted().iter().map(|n| format!("!{n}")).collect();
            format!(
                "{} candidate(s) judged; a policy would evict: {}",
                j.verdicts.len(),
                if granted.is_empty() {
                    "none".into()
                } else {
                    granted.join(", ")
                }
            )
        } else {
            "the eviction job failed; nothing is granted".into()
        };
        if let Ok(s) = platform::Settings::from(&self.env)
            && let Err(err) = super::events::append(
                &self.env,
                &s,
                "eviction.judged",
                None,
                None,
                &summary,
                false,
            )
            .await
        {
            worker::console_error!("eviction event: {err}");
        }
        e.running = None;
        e.last = Some(j);
        self.state.storage().put(EVICTIONS, &e).await?;
        Ok(false)
    }

    /// Starts the deploy of `sha` in a job Container, referencing `images`.
    async fn start(
        &self,
        p: &Pipeline,
        sha: &str,
        images: Option<&control::Images>,
    ) -> std::result::Result<(), String> {
        let (remote, token) =
            platform::credentials(&self.env, false, crate::cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let deploy = secret(&self.env, "DEPLOY_TOKEN").map_err(|e| e.to_string())?;
        let mut env = BTreeMap::from([("CLOUDFLARE_API_TOKEN".to_string(), deploy)]);
        // Only the deploy job receives these; see
        // .claude/decisions/2026-11-07-development-control-plane-deploys-itself.md §6.
        for k in crate::cobot::ship_wire::SECRETS {
            if let Ok(v) = secret(&self.env, k) {
                env.insert(k.to_string(), v);
            }
        }
        // Each platform unit's outputs come from its newest apply; see the step 4c amendment of
        // .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md.
        let kept = super::platforms::kept(&self.state)
            .await
            .map_err(|e| e.to_string())?;
        let secret_outputs = env.get(crate::cobot::ship_wire::OUTPUTS_VAR).cloned();
        if let Some(o) = crate::platforms::merged_outputs(secret_outputs.as_deref(), &kept) {
            env.insert(crate::cobot::ship_wire::OUTPUTS_VAR.to_string(), o);
        }
        // The newest images, which a deploy may have built since the secret was set; see
        // .claude/decisions/2026-11-07-development-job-images-on-cloudflare.md §4.
        if let Some(i) = images.and_then(|i| serde_json::to_string(i).ok()) {
            env.insert(crate::cobot::ship_wire::IMAGES_VAR.to_string(), i);
        }
        if let Ok(d) = self.env.var(crate::cobot::ship_wire::DEPLOYMENT_VAR) {
            env.insert(
                crate::cobot::ship_wire::DEPLOYMENT_VAR.to_string(),
                d.to_string(),
            );
        }
        if let Ok(host) = self.env.var(crate::cobot::ship_wire::HOST_VAR) {
            env.insert(
                crate::cobot::ship_wire::HOST_VAR.to_string(),
                host.to_string(),
            );
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
                    "test_map": self.test_maps().await?.view(),
                }))
            }
            (Method::Post, "ios/release") => match self.ios_release().await {
                Ok(v) => Response::from_json(&v),
                Err(why) => Response::error(why, 409),
            },
            (Method::Get, "ios/status") => Response::from_json(&self.ios_status().await?),
            (Method::Post, "evictions/run") => {
                let body: Value = req.json().await?;
                match self.evictions_run(&body["candidates"]).await {
                    Ok(v) => Response::from_json(&v),
                    Err(why) => Response::error(why, 409),
                }
            }
            (Method::Get, "evictions/last") => Response::from_json(&self.evictions().await?),
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
            let secret = secret(&self.env, crate::cobot::ship_wire::IMAGES_VAR)
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
        if let Err(e) = self.test_map_step().await {
            worker::console_error!("test map: {e}");
        }
        if let Err(e) = self.ios_step().await {
            worker::console_error!("ios release: {e}");
        }
        let evicting = match self.eviction_step().await {
            Ok(running) => running,
            Err(e) => {
                worker::console_error!("eviction: {e}");
                true
            }
        };
        let storage = self.state.storage();
        if super::platforms::running(&self.state).await? || evicting {
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

/// The first 12 characters of a SHA.
/// Decides the commit `t` holds: mapped only when it changed a Rust input since the newest
/// map's commit; see the factory ADR's 2026-11-12 amendment.
async fn decide(t: &mut TestMaps, token: &str, owner: &str, repo: &str) {
    let (Some(sha), Some(base)) = (
        t.deciding.clone(),
        t.latest_map.as_ref().map(|m| m.sha.clone()),
    ) else {
        return;
    };
    let rust = match github::compare(token, owner, repo, &base, &sha).await {
        Ok(v) => testmaps::rust_since(&v),
        Err(why) => {
            worker::console_warn!("the test map of {sha} is built: {why}");
            true
        }
    };
    *t = t.decided(&sha, rust);
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}
