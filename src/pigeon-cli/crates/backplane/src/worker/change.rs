//! The `Change` Durable Object: one change's state machine, durable in its storage. Requests
//! apply events and return at once; its alarm does the slow work (minting a read token,
//! starting and stopping job Containers, polling them) and mirrors the change to D1.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::{self, Settings, json_request, now, store_secret};
use crate::changes::{Change, Effect, Event, JobStatus};
use crate::control::{self, Outcome, Report};

const CHANGE: &str = "change";
const PENDING: &str = "pending";
const LOGS: &str = "logs";
const FAILURES: &str = "failures";
const REMOTE: &str = "remote";

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
    /// Stop the Container of this job ID.
    Stop(String),
}

/// One change; its name is `<repo>#<number>`.
#[durable_object]
pub struct ChangeRun {
    state: State,
    env: Env,
}

fn checker(env: &Env, id: &str) -> Result<worker::Stub> {
    env.durable_object("CHECKER")?.id_from_name(id)?.get_stub()
}

impl ChangeRun {
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
                        pending.extend(
                            jobs.iter()
                                .map(|j| Pending::Stop(control::job_id(change.number, sha, j))),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn save(&self, change: &Change, pending: &[Pending]) -> Result<()> {
        let storage = self.state.storage();
        storage.put(CHANGE, change).await?;
        storage.put(PENDING, pending).await?;
        let open = change
            .jobs
            .iter()
            .any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
        if !pending.is_empty() {
            storage.set_alarm(Duration::from_millis(0)).await?;
        } else if open {
            storage.set_alarm(POLL).await?;
        }
        self.mirror(change).await
    }

    /// A read token and the remote, for one job.
    async fn credentials(&self) -> Result<(String, String)> {
        let s = Settings::from(&self.env)?;
        let api = store_secret(&self.env, "ARTIFACTS_TOKEN").await?;
        let account = platform::account(&api).await?;
        let base = format!("/accounts/{account}/artifacts/namespaces/{}", s.namespace);
        let mut remote: Option<String> = self.state.storage().get(REMOTE).await?;
        if remote.is_none() {
            let repo =
                platform::cloudflare(&api, Method::Get, &format!("{base}/repos/{}", s.repo), None)
                    .await?;
            remote = cobot::artifacts_wire::remote_in(&repo);
            if let Some(r) = &remote {
                self.state.storage().put(REMOTE, r).await?;
            }
        }
        let remote = remote.ok_or("the repository's answer has no remote")?;
        let body =
            cobot::artifacts_wire::read_token_body(&s.repo, cobot::artifacts_wire::TOKEN_TTL);
        let minted =
            platform::cloudflare(&api, Method::Post, &format!("{base}/tokens"), Some(&body))
                .await?;
        let token = cobot::artifacts_wire::token_in(&minted).ok_or_else(|| {
            format!(
                "no token in the answer, whose fields are {}",
                cobot::artifacts_wire::shape(&minted)
            )
        })?;
        Ok((remote, token))
    }

    async fn start(&self, change: &Change, job: &str) -> std::result::Result<(), String> {
        let (remote, token) = self.credentials().await.map_err(|e| e.to_string())?;
        let spec = control::spec(change, job, &remote, &token)?;
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = checker(&self.env, &spec.id)
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

    async fn poll(&self, change: &Change, job: &str) -> Result<Option<Event>> {
        let Some(sha) = change.head.clone() else {
            return Ok(None);
        };
        let id = control::job_id(change.number, &sha, job);
        let mut r = checker(&self.env, &id)?
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
                Response::from_json(
                    &json!({ "change": change, "branch": change.branch(), "failures": failures }),
                )
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
        for p in std::mem::take(&mut pending) {
            match p {
                Pending::Stop(id) => {
                    if let Ok(stub) = checker(&self.env, &id) {
                        let _ = stub
                            .fetch_with_request(json_request(
                                "https://checker/stop",
                                Method::Post,
                                None,
                                None,
                            )?)
                            .await;
                    }
                }
                Pending::Start(job) => {
                    let sha = change.head.clone().unwrap_or_default();
                    let started = self.start(&change, &job).await;
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
        for job in running {
            if let Some(event) = self.poll(&change, &job).await? {
                let _ = Self::apply(&mut change, event, &mut next);
            }
        }
        self.save(&change, &next).await?;
        Response::ok("ok")
    }
}
