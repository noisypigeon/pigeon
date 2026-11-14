//! The `Change` Durable Object: one change's state machine, durable in its storage. Requests
//! apply events and return at once; its alarm does the slow work (minting a read token,
//! starting and stopping job Containers, polling them) and mirrors the change to D1.

use std::cell::RefCell;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::{self, Settings, job_stub, json_request, now};
use crate::actions::{self, Actions, Due};
use crate::changes::{Change, Effect, Event, JobStatus};
use crate::control::{self, Outcome, Report};
use crate::grants::{Grant, Grants};

const CHANGE: &str = "change";
const PENDING: &str = "pending";
/// The events applied through `/event` since the alarm last saved, which it catches up on.
const JOURNAL: &str = "journal";
const LOGS: &str = "logs";
const FAILURES: &str = "failures";

/// The storage key of the change's breaker.
const BREAKER: &str = "breaker";
/// The storage key of the last bound job's log tail, shown in its status.
const BOUND_LOG: &str = "bound_log";
const GRANTS: &str = "grants";
/// The storage key of the change's dispatches to GitHub Actions.
const ACTIONS: &str = "actions";
/// The bound job's last conflict forecast, applied once its events are.
const FORECAST: &str = "forecast";
/// The commit of the change this one is built on that its bound job last saw it on.
const AFTER_SEEN: &str = "after-seen";
/// The head's `ios` check as a macOS job.
const MACOS: &str = "macos";

mod macos_check;

/// How long an Actions token is reused, in seconds: GitHub's last an hour.
const TOKEN_REUSE: u64 = 50 * 60;

/// How often a running job is asked how it went.
const POLL: Duration = Duration::from_secs(20);

/// The most of a failed job's log `change.status` shows.
const SHOWN: usize = 8 * 1024;

/// Work for the alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Pending {
    /// Start this job on the head.
    Start(String),
    /// Stop the `Checker` Container of this job ID; queued before jobs had classes.
    Stop(String),
    /// Stop the Container of this job ID, in its class.
    StopIn(control::Class, String),
}

/// The mirror's repository on GitHub and a token for its Actions.
struct Gh<'a> {
    token: &'a str,
    owner: &'a str,
    repo: &'a str,
}

/// One change; its name is `<repo>#<number>`.
#[durable_object]
pub struct ChangeRun {
    state: State,
    env: Env,
    /// An installation token for Actions and when it was minted; in memory only.
    actions_token: RefCell<Option<(String, u64)>>,
}

impl ChangeRun {
    /// Records the owner's proof on `grant` as covering `change`'s remaining stages, when it
    /// needed one and the page named who gave it; see the 2026-11-09 approvals ADR §3.
    async fn keep_grant(&self, change: &Change, grant: &Grant, body: &Value) -> Result<()> {
        let Some(by) = body["by"].as_str() else {
            return Ok(());
        };
        if !grant.needs_code() {
            return Ok(());
        }
        let mut change = change.clone();
        change.grant = Some(crate::changes::ChangeGrant {
            by: by.to_string(),
            expires: now() + crate::grants::PAGE_TTL,
            review: body["review"].as_bool().unwrap_or_default(),
        });
        self.state.storage().put(CHANGE, &change).await
    }

    /// Sets an alarm now if work is pending or a job is open but none is set: the runtime
    /// drops an alarm that keeps failing, and only an alarm moves a job on.
    async fn rearm(&self) -> Result<()> {
        let storage = self.state.storage();
        let pending: Vec<Pending> = self.load(PENDING).await?;
        let open = storage.get::<Change>(CHANGE).await?.is_some_and(|c| {
            c.jobs
                .iter()
                .any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
        });
        if (open || !pending.is_empty()) && storage.get_alarm().await?.is_none() {
            storage.set_alarm(Duration::from_millis(0)).await?;
        }
        Ok(())
    }

    async fn load<T: for<'de> Deserialize<'de> + Default>(&self, key: &str) -> Result<T> {
        Ok(self.state.storage().get(key).await?.unwrap_or_default())
    }

    async fn mirror(&self, change: &Change) -> Result<()> {
        let s = Settings::from(&self.env)?;
        let logs: Map<String, Value> = self.load(LOGS).await?;
        platform::d1(
            &self.env,
            &control::mirror(&s.instance, &s.repo, change, &logs, now()),
        )
        .await
    }

    /// Steps `change` by `event` and keeps the result, queueing its effects for the alarm;
    /// `Cancel` becomes a stop of each job on the head it ran on. A refusal leaves it unchanged.
    fn apply(
        change: &mut Change,
        event: Event,
        pending: &mut Vec<Pending>,
    ) -> std::result::Result<(), String> {
        let head = change.head.clone();
        let (next, effects) = crate::changes::step(change, event).map_err(|r| r.0)?;
        *change = next;
        for e in effects {
            match e {
                Effect::Start(job) => pending.push(Pending::Start(job)),
                Effect::Cancel(jobs) => {
                    if let Some(sha) = &head {
                        pending.extend(jobs.iter().map(|j| {
                            Pending::StopIn(
                                control::class_of(j),
                                control::job_id(change.number, sha, j),
                            )
                        }));
                    }
                }
            }
        }
        Ok(())
    }

    /// `POST /event`: applies `event`, and journals it for an alarm already running, which
    /// would otherwise save over it.
    /// A yield set, with `to`, `by` and `reason`, or cleared with `off`; or, with `session`, the
    /// change handed to that session; see
    /// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.
    async fn park(&self, existing: Option<Change>, body: Value) -> Result<Response> {
        let Some(change) = existing else {
            return Response::error("no such change", 404);
        };
        let next = if let Some(session) = body["session"].as_str() {
            change.hand_to(session)
        } else if body["off"].as_bool() == Some(true) {
            match body["ended"].as_u64().and_then(|n| u32::try_from(n).ok()) {
                Some(to) => {
                    let head = body["head"].as_str().map(str::to_string);
                    change.unyield().after_ended(to, head)
                }
                None => change.unyield(),
            }
        } else {
            let to = body["to"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or_default();
            let by = body["by"].as_str().unwrap_or_default();
            let reason = body["reason"].as_str().unwrap_or_default();
            match change.yield_to(to, by, reason, now()) {
                Ok(c) => c,
                Err(why) => return Response::error(why, 409),
            }
        };
        let pending: Vec<Pending> = self.load(PENDING).await?;
        self.save(&next, &pending).await?;
        Response::from_json(&next)
    }

    /// The control plane's eviction: recorded, then an ordinary close; see the eviction ADR's
    /// 2026-11-13 amendment on acting.
    async fn evict(&self, existing: Option<Change>, body: Value) -> Result<Response> {
        let Some(change) = existing else {
            return Response::error("no such change", 404);
        };
        let policy = body["policy"].as_str().unwrap_or_default();
        if policy.is_empty() {
            return Response::error("an eviction names its policy", 400);
        }
        self.on_event(change.evict(policy, now()), Event::Close)
            .await
    }

    async fn on_event(&self, mut change: Change, event: Event) -> Result<Response> {
        let mut pending: Vec<Pending> = self.load(PENDING).await?;
        let asked = change.merge_after.clone();
        if let Err(why) = Self::apply(&mut change, event.clone(), &mut pending) {
            return Response::error(why, 409);
        }
        let mut journal: Vec<Event> = self.load(JOURNAL).await?;
        journal.push(event);
        if let Some(by) = asked {
            journal.extend(self.reapprove(&mut change, &by, &mut pending).await);
        }
        self.state.storage().put(JOURNAL, &journal).await?;
        self.save(&change, &pending).await?;
        Response::from_json(&change)
    }

    /// A restacked head that reached `checked` without its approval carrying, approved again by
    /// the policy or Auto window that would approve it now, and its merge, asked by `asked`, kept
    /// in its turn: the events applied, none when neither would; see the session-stages ADR's
    /// 2026-11-14 amendment.
    async fn reapprove(
        &self,
        change: &mut Change,
        asked: &str,
        pending: &mut Vec<Pending>,
    ) -> Vec<Event> {
        let dropped = change.state == crate::changes::State::Checked
            && change.approval.is_none()
            && change.merge_after.is_none();
        let Some(head) = change.head.clone().filter(|_| dropped) else {
            return Vec::new();
        };
        let by = if let Some(rule) = crate::approvals::by_policy(change, "change.approve") {
            format!("policy:{rule}")
        } else {
            let Ok(s) = Settings::from(&self.env) else {
                return Vec::new();
            };
            let window = super::mandates::window_approves(&self.env, &s, change, "change.approve");
            let Ok(Some(id)) = window.await else {
                return Vec::new();
            };
            format!("window:{id}")
        };
        let events = vec![
            Event::Approve {
                by,
                sha: head,
                at: now(),
            },
            Event::Merge {
                by: asked.to_string(),
            },
        ];
        let (mut next, mut p) = (change.clone(), pending.clone());
        for e in &events {
            if Self::apply(&mut next, e.clone(), &mut p).is_err() {
                return Vec::new();
            }
        }
        *change = next;
        *pending = p;
        events
    }

    /// Notes `change` in its breaker and logs each signal that would trip, once, as
    /// `breaker.would_trip`, at its brief's class's thresholds when it carries one; in shadow it
    /// acts on nothing. See
    /// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §4 and
    /// the model-tiered briefs ADR's §6.
    async fn breaker(&self, change: &Change) -> Result<()> {
        use crate::core::change::breaker;
        let failures: Map<String, Value> = self.load(FAILURES).await?;
        let tails: std::collections::BTreeMap<String, String> = failures
            .iter()
            .filter_map(|(job, f)| Some((job.clone(), f["log"].as_str()?.to_string())))
            .collect();
        let s = Settings::from(&self.env).ok();
        let brief = match (change.brief.as_deref(), &s) {
            (Some(id), Some(s)) => super::briefs::latest(&self.env, s, id)
                .await
                .map(|d| (id, d.model, d.budget())),
            _ => None,
        };
        let limits = brief.map_or_else(breaker::Limits::default, |(_, model, budget)| {
            breaker::Limits::brief(model, budget)
        });
        let storage = self.state.storage();
        let b: breaker::Breaker = storage.get(BREAKER).await?.unwrap_or_default();
        let b = breaker::observe(&b, change, &tails, now());
        let (b, trips) = breaker::fresh(&b, now(), &limits);
        storage.put(BREAKER, &b).await?;
        let Some(s) = s else {
            return Ok(());
        };
        for t in trips {
            let said = match brief {
                Some((id, model, _)) => format!(
                    "!{} ({}, {id}) would escalate to its planner ({}): {}",
                    change.number,
                    model.name(),
                    t.signal,
                    t.why
                ),
                None => format!(
                    "!{} would leave Auto ({}): {}",
                    change.number, t.signal, t.why
                ),
            };
            let session = change.session.clone();
            let n = Some(change.number);
            super::events::append(
                &self.env,
                &s,
                "breaker.would_trip",
                n,
                session,
                &said,
                false,
            )
            .await?;
        }
        Ok(())
    }

    async fn save(&self, change: &Change, pending: &[Pending]) -> Result<()> {
        self.save_after(change, pending, false).await
    }

    /// As [`Self::save`]; with `wait`, pending work waits for the next poll instead of running
    /// at once, as when no Container is free.
    async fn save_after(&self, change: &Change, pending: &[Pending], wait: bool) -> Result<()> {
        let storage = self.state.storage();
        let before: Option<Change> = storage.get(CHANGE).await?;
        let mut change = change.clone();
        if let Some(c) = change.conflict.as_mut()
            && let Some(p) = c.proposal.take()
        {
            match super::merges::reconcile(&self.env, change.number, change.session.clone(), p)
                .await
            {
                Ok(n) => c.reconcile = n,
                Err(e) => worker::console_error!("reconcile: {e}"),
            }
        }
        let change = &change;
        storage.put(CHANGE, change).await?;
        // D1 first: what follows reads it; see the concurrent-sessions ADR's 2026-11-13 amendment
        // on generated work following its parent's own record.
        self.mirror(change).await?;
        super::events::changed(&self.env, before.as_ref(), change).await;
        if let Err(e) = self.breaker(change).await {
            worker::console_error!("breaker for !{}: {e}", change.number);
        }
        let gone = crate::reconcile::gone(before.as_ref(), change);
        if let Some(ended) = gone
            && let Err(e) = super::merges::retire(&self.env, change.number, ended).await
        {
            worker::console_error!("retire for !{}: {e}", change.number);
        }
        if gone == Some(true)
            && let Err(e) = self.ended_effects(change).await
        {
            worker::console_error!("ended !{}: {e}", change.number);
        }
        if control::leaves_merge_queue(before.as_ref(), change) {
            self.released(change).await;
            super::merges::ended(&self.env, change, None).await?;
        }
        let mut pending = pending.to_vec();
        if control::unmirror_due(before.as_ref(), change) {
            pending.push(Pending::Start(control::UNMIRROR.into()));
        }
        let pending = &pending;
        storage.put(PENDING, pending).await?;
        let open = change
            .jobs
            .iter()
            .any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
        if !pending.is_empty() && !wait {
            storage.set_alarm(Duration::from_millis(0)).await?;
        } else if open || !pending.is_empty() {
            storage.set_alarm(POLL).await?;
        }
        let found = before.as_ref().and_then(|b| b.landed.as_ref());
        if change.landed.is_some() && found != change.landed.as_ref() {
            self.close_landed(change).await;
        }
        Ok(())
    }

    /// What a merged or closed `change` ends: its yields, its generated changes, and its tasks.
    async fn ended_effects(&self, change: &Change) -> Result<()> {
        super::release_yields(&self.env, change).await?;
        let s = Settings::from(&self.env)?;
        super::tasks::close_for(&self.env, &s, change.number).await
    }

    /// `POST /heal`: a merged or closed change's D1 row and endings again, for a row D1 still
    /// shows open; see the concurrent-sessions ADR's 2026-11-13 amendment.
    async fn heal(&self, existing: Option<Change>) -> Result<Response> {
        let done = |c: &Change| {
            use crate::changes::State as S;
            matches!(c.state, S::Merged | S::Closed)
        };
        let Some(change) = existing.filter(done) else {
            return Response::error("not a merged or closed change", 409);
        };
        self.mirror(&change).await?;
        super::merges::retire(&self.env, change.number, true).await?;
        self.ended_effects(&change).await?;
        Response::from_json(&change)
    }

    /// Asks to close `change`, whose patch already landed on `main`: an Auto window approves it,
    /// otherwise the owner does on the approval page; see
    /// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §4 F3.
    async fn close_landed(&self, change: &Change) {
        let Ok(s) = Settings::from(&self.env) else {
            return;
        };
        let n = change.number;
        let said = format!("!{n}'s patch is already on main; asking to close it");
        let added = super::events::append(
            &self.env,
            &s,
            "change.landed",
            Some(n),
            change.session.clone(),
            &said,
            false,
        );
        if let Err(e) = added.await {
            worker::console_error!("change.landed: {e}");
        }
        let mut args = Map::new();
        args.insert("number".into(), json!(n));
        let request = crate::wire::Request {
            operation: "change.close".into(),
            args,
        };
        match super::ask(&self.env, &s, n, &request).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => worker::console_warn!("close of landed !{n}: {e}"),
            Err(e) => worker::console_error!("close of landed !{n}: {e}"),
        }
    }

    /// Records `merge.released`: `change` gave its merge turn back without merging.
    async fn released(&self, change: &Change) {
        let Ok(s) = Settings::from(&self.env) else {
            return;
        };
        let why = format!(
            "!{} gave its merge turn back: {}",
            change.number,
            control::release_reason(change)
        );
        let n = Some(change.number);
        let session = change.session.clone();
        let added = super::events::append(&self.env, &s, "merge.released", n, session, &why, false);
        if let Err(e) = added.await {
            worker::console_error!("merge.released: {e}");
        }
    }

    async fn start(&self, change: &Change, job: &str) -> std::result::Result<(), String> {
        let write = job == crate::changes::MERGE || job == crate::changes::RESTACK;
        let (remote, token) =
            platform::credentials(&self.env, write, crate::cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let mut spec = control::spec(change, job, &remote, &token)?;
        if job == crate::changes::BOUND {
            let overlaps = super::overlapping(&self.env, change)
                .await
                .map_err(|e| e.to_string())?;
            spec.env
                .insert(control::OVERLAPS.into(), overlaps.to_string());
            if let Some(b) = change.built_on() {
                spec.env.insert(control::AFTER.into(), b.to_string());
            }
        }
        // Past the commit the bound job saw it built on, which survives a restack of the change
        // it is built on; else that change's last head. See the yield ADR's 2026-11-14 amendment.
        if job == crate::changes::RESTACK && change.after.is_some() {
            let seen: Option<String> = self.state.storage().get(AFTER_SEEN).await.ok().flatten();
            let last = change.after.as_ref().and_then(|a| a.head.clone());
            if let Some(h) = seen.or(last) {
                spec.env.insert(control::AFTER_HEAD.into(), h);
            }
        }
        if job == control::MIRROR {
            let (to, to_token) = self.github().await?;
            spec.env.insert(control::MIRROR_TO.into(), to);
            spec.env.insert(control::MIRROR_TOKEN.into(), to_token);
        }
        if job == crate::changes::MERGE
            && let Ok(author) = self.env.var(control::COMMIT_AUTHOR)
            && !author.to_string().is_empty()
        {
            spec.env
                .insert(control::COMMIT_AUTHOR.into(), author.to_string());
        }
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = job_stub(&self.env, control::class_of(job), &spec.id)
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

    /// The GitHub mirror and an installation token for it, minted for one job.
    async fn github(&self) -> std::result::Result<(String, String), String> {
        let to = super::github::mirror(&self.env).ok_or("this Worker has no GitHub mirror")?;
        let (owner, repo) = crate::mirroring::github_repo(&to)
            .ok_or_else(|| format!("the mirror `{to}` is not a GitHub repository"))?;
        let token = super::github::installation_token(
            &self.env,
            &owner,
            &repo,
            &crate::mirroring::token_request(&repo),
        )
        .await?;
        Ok((to, token))
    }

    /// Starts the job deleting `change`'s branch from GitHub; nothing polls it.
    async fn unmirror(&self, change: &Change) -> std::result::Result<(), String> {
        let (to, to_token) = self.github().await?;
        let spec = control::unmirror_spec(change, &to, &to_token);
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = job_stub(&self.env, control::class_of(control::UNMIRROR), &spec.id)
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

    /// An installation token for Actions on the mirror, reused for [`TOKEN_REUSE`].
    async fn actions_token(&self, owner: &str, repo: &str) -> std::result::Result<String, String> {
        let at = now();
        if let Some((t, minted)) = self.actions_token.borrow().clone()
            && at < minted + TOKEN_REUSE
        {
            return Ok(t);
        }
        let t = super::github::installation_token(
            &self.env,
            owner,
            repo,
            &crate::mirroring::actions_token_request(repo),
        )
        .await?;
        *self.actions_token.borrow_mut() = Some((t.clone(), at));
        Ok(t)
    }

    /// The head's code checks on GitHub, then its `ios` check wherever it runs.
    async fn ci_step(
        &self,
        change: &mut Change,
        github: &mut Actions,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        self.github_step(change, github, next).await?;
        self.macos_step(change, github, next).await
    }

    /// Carries out what `github` says is due: re-runs the mirror job, dispatches `ci.yml`, finds
    /// and polls runs, and applies each finished check with its log; see
    /// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §3–§5.
    async fn github_step(
        &self,
        change: &mut Change,
        github: &mut Actions,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        let at = now();
        let open: Vec<String> = change
            .jobs
            .iter()
            .filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
            .map(|j| j.name.clone())
            .collect();
        github.keep_open(&open);
        let due = github.due(control::mirror_state(change), at);
        if due.contains(&Due::Mirror) {
            let _ = Self::apply(
                change,
                Event::Rerun {
                    job: control::MIRROR.into(),
                },
                next,
            );
        }
        let calls: Vec<Due> = due.into_iter().filter(|d| *d != Due::Mirror).collect();
        let (Some(sha), false) = (change.head.clone(), calls.is_empty()) else {
            return Ok(());
        };
        let fail_all = |github: &mut Actions, why: &str| {
            for d in &calls {
                if let Due::Dispatch(i) | Due::Find(i) | Due::Poll(i) = d {
                    github.failed(*i, why, at);
                }
            }
        };
        let Some((owner, repo)) =
            super::github::mirror(&self.env).and_then(|to| crate::mirroring::github_repo(&to))
        else {
            fail_all(github, "this Worker has no GitHub mirror");
            return Ok(());
        };
        let token = match self.actions_token(&owner, &repo).await {
            Ok(t) => t,
            Err(why) => {
                fail_all(github, &why);
                return Ok(());
            }
        };
        for d in calls {
            match d {
                Due::Dispatch(i) => {
                    let body = actions::dispatch_body(
                        &github.dispatches[i],
                        change.number,
                        &control::checks_of(change),
                    );
                    match super::github::dispatch(&token, &owner, &repo, &body).await {
                        Ok(()) => {
                            for job in github.dispatched(i, at) {
                                let _ = Self::apply(
                                    change,
                                    Event::Started {
                                        job,
                                        sha: sha.clone(),
                                    },
                                    next,
                                );
                            }
                        }
                        Err(why) => github.failed(i, &why, at),
                    }
                }
                Due::Find(i) => match super::github::runs(&token, &owner, &repo).await {
                    Ok(listed) => github.found(i, &listed, at),
                    Err(why) => github.failed(i, &why, at),
                },
                Due::Poll(i) => {
                    let gh = Gh {
                        token: &token,
                        owner: &owner,
                        repo: &repo,
                    };
                    self.github_poll(&gh, change, github, i, next).await?;
                }
                Due::Mirror => {}
            }
        }
        Ok(())
    }

    /// Polls entry `i`'s run, applying each finished check with its log.
    async fn github_poll(
        &self,
        gh: &Gh<'_>,
        change: &mut Change,
        github: &mut Actions,
        i: usize,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        let (Some(run), Some(sha)) = (github.dispatches[i].run, change.head.clone()) else {
            return Ok(());
        };
        let at = now();
        let url = github.dispatches[i].url.clone().unwrap_or_default();
        let (r, jobs) = match super::github::run(gh.token, gh.owner, gh.repo, run).await {
            Ok(v) => v,
            Err(why) => {
                github.failed(i, &why, at);
                return Ok(());
            }
        };
        self.macos_github_polled(github, i, &jobs).await?;
        for f in github.polled(i, &r, &jobs, at) {
            let log = match f.job {
                Some(id) => super::github::job_log(gh.token, gh.owner, gh.repo, id)
                    .await
                    .unwrap_or_else(|e| format!("the log is unavailable: {e}\n{url}\n")),
                None => format!("the run completed without this check: {url}\n"),
            };
            let outcome = if f.ok {
                Outcome::Passed
            } else {
                Outcome::Failed
            };
            let id = control::job_id(change.number, &sha, &f.name);
            self.keep_log(&f.name, &id, &Report::of(&id, outcome, &log))
                .await?;
            let _ = Self::apply(
                change,
                Event::Finished {
                    job: f.name,
                    sha: sha.clone(),
                    ok: f.ok,
                },
                next,
            );
        }
        Ok(())
    }

    /// Starts `job`'s Container, applying `Started`, or `Lost` when it didn't start, which joins
    /// `applied`; false when it waits for a free Container, queued again.
    async fn start_container(
        &self,
        change: &mut Change,
        github: &Actions,
        job: String,
        next: &mut Vec<Pending>,
        applied: &mut Vec<Event>,
    ) -> bool {
        let sha = change.head.clone().unwrap_or_default();
        let running = change
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Running && !github.owns(&sha, &j.name))
            .count();
        if !control::may_start(running) {
            next.push(Pending::Start(job));
            return false;
        }
        let started = self.start(change, &job).await;
        if let Err(why) = &started
            && control::no_capacity(why)
        {
            worker::console_log!("job {job} waits for a free Container");
            next.push(Pending::Start(job));
            return false;
        }
        let _ = Self::apply(
            change,
            Event::Started {
                job: job.clone(),
                sha: sha.clone(),
            },
            next,
        );
        if let Err(why) = started {
            worker::console_warn!("job {job} did not start: {why}");
            let lost = Event::Lost { job, sha };
            if Self::apply(change, lost.clone(), next).is_ok() {
                applied.push(lost);
            }
        }
        true
    }

    /// The CI window opened: the break-glass, when the change has code checks waiting on GitHub.
    async fn window(&self, existing: Option<Change>) -> Result<Response> {
        match existing {
            Some(change) if control::window_moves(&change) => {
                self.break_glass(change).await?;
                Response::from_json(&json!({ "moved": true }))
            }
            Some(_) => Response::from_json(&json!({ "moved": false })),
            None => Response::error("no such change", 404),
        }
    }

    /// The owner's break-glass: the head's code checks that haven't passed, on the `Checker`.
    async fn break_glass(&self, mut change: Change) -> Result<Response> {
        let mut github: Actions = self.load(ACTIONS).await?;
        let (events, start) = match control::break_glass(&change, &mut github) {
            Ok(v) => v,
            Err(why) => return Response::error(why, 409),
        };
        let mut pending: Vec<Pending> = self.load(PENDING).await?;
        for e in events {
            let _ = Self::apply(&mut change, e, &mut pending);
        }
        pending.extend(start.into_iter().map(Pending::Start));
        self.state.storage().put(ACTIONS, &github).await?;
        self.save(&change, &pending).await?;
        Response::from_json(&change)
    }

    /// Whether the owner's CI window is open now; closed when the `Instance` doesn't answer.
    async fn window_open(&self) -> bool {
        let Ok(s) = Settings::from(&self.env) else {
            return false;
        };
        match super::ci_window::current(&self.env, &s).await {
            Ok(w) => w.is_some(),
            Err(e) => {
                worker::console_warn!("ci window: {e}");
                false
            }
        }
    }

    /// Stops job `id`'s Container in `class`, if it still runs.
    async fn stop(&self, class: control::Class, id: &str) -> Result<()> {
        if let Ok(stub) = job_stub(&self.env, class, id) {
            let _ = stub
                .fetch_with_request(json_request(
                    "https://checker/stop",
                    Method::Post,
                    None,
                    None,
                )?)
                .await;
        }
        Ok(())
    }

    async fn poll(&self, change: &Change, job: &str) -> Result<Option<Event>> {
        let Some(sha) = change.head.clone() else {
            return Ok(None);
        };
        let id = control::job_id(change.number, &sha, job);
        let mut r = job_stub(&self.env, control::class_of(job), &id)?
            .fetch_with_str(&format!("https://checker/status/{id}"))
            .await?;
        match r.status_code() {
            200 => {
                let report: Report = r.json().await?;
                if report.outcome != Outcome::Running {
                    self.keep_log(job, &id, &report).await?;
                }
                if job == crate::changes::BOUND && !report.forecast.is_empty() {
                    self.state.storage().put(FORECAST, &report.forecast).await?;
                }
                if job == crate::changes::BOUND
                    && let Some(seen) = &report.built_on
                {
                    self.state.storage().put(AFTER_SEEN, seen).await?;
                }
                let event = control::event_of(job, &sha, &report);
                Ok(if super::github::mirror(&self.env).is_some() {
                    event.map(control::with_mirror)
                } else {
                    event
                })
            }
            404 => Ok(Some(Event::Lost {
                job: job.into(),
                sha,
            })),
            _ => Ok(None),
        }
    }

    /// The bound job's kept forecast, applied to `change` once; see the 2026-11-12 ADR §5.
    async fn forecast_step(&self, change: Change) -> Result<Change> {
        let forecast: Vec<Value> = self.load(FORECAST).await?;
        if forecast.is_empty() {
            return Ok(change);
        }
        self.state.storage().delete(FORECAST).await?;
        super::apply_forecast(&self.env, change, &forecast).await
    }

    async fn keep_log(&self, job: &str, id: &str, report: &Report) -> Result<()> {
        let key = format!("ci/{id}.log");
        self.env
            .bucket("CI_LOGS")?
            .put(key.clone(), control::tail(&report.log).to_string())
            .execute()
            .await?;
        let storage = self.state.storage();
        let mut logs: Map<String, Value> = self.load(LOGS).await?;
        logs.insert(job.into(), key.into());
        storage.put(LOGS, &logs).await?;
        if job == crate::changes::BOUND {
            storage
                .put(BOUND_LOG, &control::last(&report.log, SHOWN).to_string())
                .await?;
        }
        let mut failures: Map<String, Value> = self.load(FAILURES).await?;
        if report.outcome == Outcome::Failed {
            let log = &report.log;
            let shown = control::last(log, SHOWN);
            failures.insert(
                job.into(),
                json!({ "log": shown, "violations": report.violations }),
            );
        } else {
            failures.remove(job);
        }
        storage.put(FAILURES, &failures).await
    }
}

impl DurableObject for ChangeRun {
    fn new(state: State, env: Env) -> Self {
        Self {
            state,
            env,
            actions_token: RefCell::new(None),
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per route")]
    async fn fetch(&self, mut req: Request) -> Result<Response> {
        self.rearm().await?;
        let path = req.path();
        let existing: Option<Change> = self.state.storage().get(CHANGE).await?;
        match (req.method(), path.trim_matches('/')) {
            (Method::Post, "propose") => {
                if existing.is_some() {
                    return Response::error("this change number is taken", 409);
                }
                let change: Change = req.json().await?;
                self.save(&change, &[]).await?;
                Response::from_json(&change)
            }
            (Method::Get, "status") => {
                let Some(change) = existing else {
                    return Response::error("no such change", 404);
                };
                let failures: Map<String, Value> = self.load(FAILURES).await?;
                let bound_log: Option<String> = self.state.storage().get(BOUND_LOG).await?;
                let github: Actions = self.load(ACTIONS).await?;
                let macos: Option<crate::macos::Job> = self.load(MACOS).await?;
                Response::from_json(&json!({
                    "macos": macos.as_ref().map(crate::macos::view),
                    "change": change,
                    "branch": change.branch(),
                    "failures": failures,
                    "bound_log": bound_log,
                    "github": github.view(),
                    "gate": match change.gate_why() {
                        None => json!({ "mode": "scoped" }),
                        Some(why) => json!({ "mode": "full", "why": why }),
                    },
                }))
            }
            (Method::Post, "grant") => {
                let Some(change) = existing else {
                    return Response::error("no such change", 404);
                };
                let body: Value = req.json().await?;
                let operation = body["operation"].as_str().unwrap_or_default();
                let token = platform::random_hex(32)?;
                let grants: Grants = self.load(GRANTS).await?;
                let ttl = if body["page"].as_bool() == Some(true) {
                    crate::grants::PAGE_TTL
                } else {
                    crate::grants::TTL
                };
                let (grants, minted) = grants.mint_for(&change, operation, &token, now(), ttl);
                match minted {
                    Ok(grant) => {
                        self.state.storage().put(GRANTS, &grants).await?;
                        Response::from_json(
                            &json!({ "token": token, "grant": grant, "title": change.title }),
                        )
                    }
                    Err(why) => Response::error(why, 409),
                }
            }
            (Method::Post, "peek") => {
                let Some(change) = existing else {
                    return Response::error("no such change", 404);
                };
                let body: Value = req.json().await?;
                let token = body["token"].as_str().unwrap_or_default();
                let grants: Grants = self.load(GRANTS).await?;
                match grants.peek(token, change.number, now()) {
                    Ok(grant) => Response::from_json(&json!({ "grant": grant, "change": change })),
                    Err(why) => Response::error(why, 403),
                }
            }
            (Method::Post, "redeem") => {
                let Some(change) = existing else {
                    return Response::error("no such change", 404);
                };
                let body: Value = req.json().await?;
                let token = body["token"].as_str().unwrap_or_default();
                let operation = body["operation"].as_str().unwrap_or_default();
                let grants: Grants = self.load(GRANTS).await?;
                let (grants, redeemed) = grants.redeem(token, operation, &change, now());
                self.state.storage().put(GRANTS, &grants).await?;
                if let Ok(g) = &redeemed {
                    self.keep_grant(&change, g, &body).await?;
                }
                match redeemed {
                    Ok(grant) => Response::from_json(&grant),
                    Err(why) => Response::error(why, 403),
                }
            }
            (Method::Post, "window") => self.window(existing).await,
            (Method::Post, "cloudflare") => match existing {
                Some(change) => self.break_glass(change).await,
                None => Response::error("no such change", 404),
            },
            (Method::Post, "event") => match existing {
                Some(change) => self.on_event(change, req.json().await?).await,
                None => Response::error("no such change", 404),
            },
            (Method::Post, "evict") => self.evict(existing, req.json().await?).await,
            (Method::Post, "heal") => self.heal(existing).await,
            (Method::Post, "yield" | "session") => self.park(existing, req.json().await?).await,
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let Some(mut change) = self.state.storage().get::<Change>(CHANGE).await? else {
            return Response::ok("no change");
        };
        let mut pending: Vec<Pending> = self.load(PENDING).await?;
        // What `/event` may add while this alarm awaits, which it keeps rather than overwrites.
        let seen = self.load::<Vec<Event>>(JOURNAL).await?.len();
        let took = pending.len();
        let mut next = Vec::new();
        let mut wait = false;
        let mut applied = Vec::new();
        let mut github: Actions = self.load(ACTIONS).await?;
        github.keep_head(change.head.as_deref());
        let mirrors = super::github::mirror(&self.env).is_some();
        let window_open = mirrors
            && pending.iter().any(
                |p| matches!(p, Pending::Start(j) if control::to_github(&change, j, true, false)),
            )
            && self.window_open().await;
        for p in std::mem::take(&mut pending) {
            match p {
                Pending::Stop(id) => self.stop(control::Class::Checker, &id).await?,
                Pending::StopIn(class, id) => self.stop(class, &id).await?,
                Pending::Start(job) if job == control::UNMIRROR => {
                    match self.unmirror(&change).await {
                        Err(why) if control::no_capacity(&why) => {
                            next.push(Pending::Start(job));
                            wait = true;
                        }
                        Err(why) => worker::console_warn!("unmirror {}: {why}", change.branch()),
                        Ok(()) => {}
                    }
                }
                Pending::Start(job) if mirrors && macos_check::is_ios(&change, &job) => {
                    self.macos_start(&mut change, &mut github, window_open, &mut next)
                        .await?;
                }
                Pending::Start(job)
                    if control::to_github(&change, &job, mirrors, window_open)
                        && !github.on_cloudflare(change.head.as_deref().unwrap_or_default()) =>
                {
                    if let Some(sha) = &change.head {
                        github.queue(change.number, sha, &job);
                    }
                }
                Pending::Start(job) => {
                    if !self
                        .start_container(&mut change, &github, job, &mut next, &mut applied)
                        .await
                    {
                        wait = true;
                    }
                }
            }
        }
        let head = change.head.clone().unwrap_or_default();
        let running: Vec<String> = change
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Running && !github.owns(&head, &j.name))
            .map(|j| j.name.clone())
            .collect();
        let restack_status = |c: &Change| {
            c.jobs
                .iter()
                .find(|j| j.name == crate::changes::RESTACK)
                .map(|j| j.status)
        };
        let restack_before = restack_status(&change);
        for job in running {
            if let Some(event) = self.poll(&change, &job).await?
                && Self::apply(&mut change, event.clone(), &mut next).is_ok()
            {
                applied.push(event);
            }
        }
        // Finished failing or lost on its last attempt; see the 2026-11-13 amendment of
        // .claude/decisions/2026-11-08-development-concurrent-agent-sessions.md.
        let restack_failed = restack_before != Some(JobStatus::Failed)
            && restack_status(&change) == Some(JobStatus::Failed);
        let ended = control::merge_ended_by(&applied, &change);
        self.ci_step(&mut change, &mut github, &mut next).await?;
        self.state.storage().put(ACTIONS, &github).await?;
        let journal: Vec<Event> = self.load(JOURNAL).await?;
        control::catch_up(&mut change, &journal, seen);
        change = self.forecast_step(change).await?;
        let stored: Vec<Pending> = self.load(PENDING).await?;
        next.extend(stored.into_iter().skip(took));
        self.state
            .storage()
            .put(JOURNAL, &Vec::<Event>::new())
            .await?;
        self.save_after(&change, &next, wait).await?;
        self.alarm_done(&change, restack_failed, ended).await;
        Response::ok("ok")
    }
}

impl ChangeRun {
    /// After an alarm saved `change`: a failed restack's task, and the merge queue told when its
    /// merge ended.
    #[allow(
        clippy::option_option,
        reason = "`control::merge_ended_by`'s answer, passed on"
    )]
    async fn alarm_done(
        &self,
        change: &Change,
        restack_failed: bool,
        ended: Option<Option<String>>,
    ) {
        if restack_failed && let Err(e) = super::tasks::restack_failed(&self.env, change).await {
            worker::console_error!("restack task: {e}");
        }
        if control::released_without_merge(ended.as_ref()) {
            self.released(change).await;
        }
        if let Some(merged) = ended
            && let Err(e) = super::merges::ended(&self.env, change, merged.as_deref()).await
        {
            worker::console_error!("merge queue: {e}");
        }
    }
}
