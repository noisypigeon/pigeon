//! The `Change` Durable Object: one change's state machine, durable in its storage. Requests
//! apply events and return at once; its alarm does the slow work (minting a read token,
//! starting and stopping job Containers, polling them) and mirrors the change to D1.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::{self, Settings, job_stub, json_request, now};
use crate::changes::{Change, Effect, Event, JobStatus};
use crate::control::{self, Outcome, Report};
use crate::grants::Grants;

const CHANGE: &str = "change";
const PENDING: &str = "pending";
const LOGS: &str = "logs";
const FAILURES: &str = "failures";
/// The storage key of the last bound job's log tail, shown in its status.
const BOUND_LOG: &str = "bound_log";
const GRANTS: &str = "grants";

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

/// One change; its name is `<repo>#<number>`.
#[durable_object]
pub struct ChangeRun {
    state: State,
    env: Env,
}

impl ChangeRun {
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

    /// Applies `event`, queueing its effects for the alarm; `Cancel` becomes a stop of each
    /// job on the head it ran on.
    fn apply(
        change: &mut Change,
        event: Event,
        pending: &mut Vec<Pending>,
    ) -> std::result::Result<(), String> {
        let head = change.head.clone();
        let effects = change.apply(event).map_err(|r| r.0)?;
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

    async fn save(&self, change: &Change, pending: &[Pending]) -> Result<()> {
        self.save_after(change, pending, false).await
    }

    /// As [`Self::save`]; with `wait`, pending work waits for the next poll instead of running
    /// at once, as when no Container is free.
    async fn save_after(&self, change: &Change, pending: &[Pending], wait: bool) -> Result<()> {
        let storage = self.state.storage();
        let before: Option<Change> = storage.get(CHANGE).await?;
        storage.put(CHANGE, change).await?;
        if before.is_some_and(|b| b.merge_after.is_some()) && control::merge_dropped(change) {
            super::merges::ended(&self.env, change.number, None).await?;
        }
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
        self.mirror(change).await
    }

    async fn start(&self, change: &Change, job: &str) -> std::result::Result<(), String> {
        let write = job == crate::changes::MERGE || job == crate::changes::RESTACK;
        let (remote, token) =
            platform::credentials(&self.env, write, cobot::artifacts_wire::TOKEN_TTL)
                .await
                .map_err(|e| e.to_string())?;
        let spec = control::spec(change, job, &remote, &token)?;
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
                Ok(control::event_of(job, &sha, &report))
            }
            404 => Ok(Some(Event::Lost {
                job: job.into(),
                sha,
            })),
            _ => Ok(None),
        }
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
        Self { state, env }
    }

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
                Response::from_json(&json!({
                    "change": change,
                    "branch": change.branch(),
                    "failures": failures,
                    "bound_log": bound_log,
                }))
            }
            (Method::Post, "grant") => {
                let Some(change) = existing else {
                    return Response::error("no such change", 404);
                };
                let body: Value = req.json().await?;
                let operation = body["operation"].as_str().unwrap_or_default();
                let token = platform::random_hex(32)?;
                let mut grants: Grants = self.load(GRANTS).await?;
                let ttl = if body["page"].as_bool() == Some(true) {
                    crate::grants::PAGE_TTL
                } else {
                    crate::grants::TTL
                };
                match grants.mint_for(&change, operation, &token, now(), ttl) {
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
                let mut grants: Grants = self.load(GRANTS).await?;
                let redeemed = grants.redeem(token, operation, &change, now());
                self.state.storage().put(GRANTS, &grants).await?;
                match redeemed {
                    Ok(grant) => Response::from_json(&grant),
                    Err(why) => Response::error(why, 403),
                }
            }
            (Method::Post, "event") => {
                let Some(mut change) = existing else {
                    return Response::error("no such change", 404);
                };
                let event: Event = req.json().await?;
                let mut pending: Vec<Pending> = self.load(PENDING).await?;
                if let Err(why) = Self::apply(&mut change, event, &mut pending) {
                    return Response::error(why, 409);
                }
                self.save(&change, &pending).await?;
                Response::from_json(&change)
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let Some(mut change) = self.state.storage().get::<Change>(CHANGE).await? else {
            return Response::ok("no change");
        };
        let mut pending: Vec<Pending> = self.load(PENDING).await?;
        let mut next = Vec::new();
        let mut wait = false;
        for p in std::mem::take(&mut pending) {
            match p {
                Pending::Stop(id) => self.stop(control::Class::Checker, &id).await?,
                Pending::StopIn(class, id) => self.stop(class, &id).await?,
                Pending::Start(job) => {
                    let sha = change.head.clone().unwrap_or_default();
                    let running = change
                        .jobs
                        .iter()
                        .filter(|j| j.status == JobStatus::Running)
                        .count();
                    if !control::may_start(running) {
                        next.push(Pending::Start(job));
                        wait = true;
                        continue;
                    }
                    let started = self.start(&change, &job).await;
                    if let Err(why) = &started
                        && control::no_capacity(why)
                    {
                        worker::console_log!("job {job} waits for a free Container");
                        next.push(Pending::Start(job));
                        wait = true;
                        continue;
                    }
                    let _ = Self::apply(
                        &mut change,
                        Event::Started {
                            job: job.clone(),
                            sha: sha.clone(),
                        },
                        &mut next,
                    );
                    if let Err(why) = started {
                        worker::console_warn!("job {job} did not start: {why}");
                        let _ = Self::apply(&mut change, Event::Lost { job, sha }, &mut next);
                    }
                }
            }
        }
        let running: Vec<String> = change
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Running)
            .map(|j| j.name.clone())
            .collect();
        let mut ended: Option<Option<String>> = None;
        for job in running {
            if let Some(event) = self.poll(&change, &job).await? {
                let merge = match &event {
                    Event::Merged { commit, .. } => Some(commit.clone()),
                    Event::MergeFailed { .. } => Some(None),
                    _ => None,
                };
                if Self::apply(&mut change, event, &mut next).is_ok() && merge.is_some() {
                    ended = merge;
                }
            }
        }
        self.save_after(&change, &next, wait).await?;
        if let Some(merged) = ended
            && let Err(e) = super::merges::ended(&self.env, change.number, merged.as_deref()).await
        {
            worker::console_error!("merge queue: {e}");
        }
        Response::ok("ok")
    }
}
