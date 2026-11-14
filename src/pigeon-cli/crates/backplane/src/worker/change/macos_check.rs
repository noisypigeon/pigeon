//! A change head's `ios` check as a macOS job: placed on GitHub, Bitrise or Buildkite, moved
//! when a provider gives no machine, and its result applied; see
//! `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §2.

use serde_json::Value;
use worker::Result;

use super::super::platform::{self, Settings, now};
use super::{ChangeRun, MACOS, Pending};
use crate::actions::{self, Actions};
use crate::changes::{Change, Event, JobStatus};
use crate::control::{self, Outcome as JobOutcome, Report};
use crate::macos::{self, Job, Kind, Outcome, Placed, Provider, Settled};

/// The `ios` check's name, as `cobot::scope` names it.
pub const IOS: &str = "ios";

/// Whether `job` on `change` is its `ios` check.
pub fn is_ios(change: &Change, job: &str) -> bool {
    job == IOS
        && change
            .jobs
            .iter()
            .any(|j| j.name == IOS && j.argv == ["bash", actions::IOS_CHECK])
}

impl ChangeRun {
    /// Starts head `sha`'s `ios` check as a macOS job, GitHub skipped while the owner's CI window
    /// is open or the head's checks were moved to Cloudflare; any earlier one is stopped.
    pub(super) async fn macos_start(
        &self,
        change: &mut Change,
        github: &mut Actions,
        window_open: bool,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        let Some(sha) = change.head.clone() else {
            return Ok(());
        };
        let old: Option<Job> = self.load(MACOS).await?;
        if let Some(old) = old
            && let Some(a) = old.current()
        {
            self.macos_abandon(github, &old, a).await;
        }
        let skip_github = window_open || github.on_cloudflare(&sha);
        let skip = if skip_github {
            vec![Provider::Github]
        } else {
            Vec::new()
        };
        let job = Job::new(
            Kind::Check,
            u64::from(change.number),
            &sha,
            now(),
            None,
            skip,
        );
        let _ = Self::apply(
            change,
            Event::Started {
                job: IOS.into(),
                sha,
            },
            next,
        );
        self.state.storage().put(MACOS, &Some(job)).await
    }

    /// Stops attempt `a` of `job` where it runs: GitHub's is dropped from its dispatch, which
    /// can't cancel one job of a run; Bitrise's is aborted and Buildkite's cancelled.
    async fn macos_abandon(&self, github: &mut Actions, job: &Job, a: &macos::Attempt) {
        match (a.provider, a.id.as_deref()) {
            (Provider::Github, _) => github.drop_check(&job.sha, IOS),
            (p, Some(id)) => super::super::macos::abandon(&self.env, p, id).await,
            (_, None) => {}
        }
    }

    /// Reads the `ios` job of GitHub run `jobs` into the head's macOS job, when it runs there:
    /// an `ios` job GitHub gave no machine, or lost, is dropped from the dispatch so its result
    /// is never applied.
    pub(super) async fn macos_github_polled(
        &self,
        github: &mut Actions,
        i: usize,
        jobs: &Value,
    ) -> Result<()> {
        let Some(mut job): Option<Job> = self.load(MACOS).await? else {
            return Ok(());
        };
        let d = &github.dispatches[i];
        let ours = d.sha == job.sha
            && d.checks.iter().any(|c| c == IOS)
            && !d.reported.iter().any(|c| c == IOS);
        let Some(a) = job
            .current()
            .filter(|a| ours && a.provider == Provider::Github)
            .cloned()
        else {
            return Ok(());
        };
        let at = now();
        if a.id.is_none()
            && let Some(run) = d.run
        {
            job = macos::dispatched(&job, &run.to_string(), d.url.as_deref());
        }
        let from = d.dispatched_at.unwrap_or(a.queued_at);
        let start_within = macos::config().map_or(600, |c| c.start_within);
        let entry = jobs["jobs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|j| j["name"] == IOS)
            .cloned()
            .unwrap_or_default();
        let o = macos::github(&entry, IOS, from, at, start_within);
        if matches!(o, Outcome::Unassigned | Outcome::Lost) {
            github.drop_check(&job.sha, IOS);
        }
        job = macos::observed(&job, o, at);
        self.state.storage().put(MACOS, &Some(job)).await
    }

    /// Moves the head's macOS job on: a GitHub attempt that was never dispatched, or whose run
    /// never appeared, past `start_within`; a Bitrise or Buildkite attempt driven; its settling,
    /// with a `ci.moved` event when it moves; and its placing.
    pub(super) async fn macos_step(
        &self,
        change: &mut Change,
        github: &mut Actions,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        let Some(before): Option<Job> = self.load(MACOS).await? else {
            return Ok(());
        };
        let running = change
            .jobs
            .iter()
            .any(|j| j.name == IOS && j.status == JobStatus::Running);
        if before.result.is_some()
            || change.head.as_deref() != Some(before.sha.as_str())
            || !running
        {
            if before.result.is_none()
                && let Some(a) = before.current()
            {
                self.macos_abandon(github, &before, a).await;
            }
            return self.state.storage().put(MACOS, &None::<Job>).await;
        }
        let Ok(cfg) = macos::config() else {
            return Ok(());
        };
        let s = Settings::from(&self.env)?;
        let at = now();
        let mut job = before.clone();
        if let Some(a) = job.current().cloned() {
            job = if a.provider == Provider::Github {
                github_unstarted(github, &job, cfg.start_within, at)
            } else {
                super::super::macos::drive(&self.env, &job, &change.branch(), cfg.start_within, at)
                    .await
            };
        }
        let (settled_job, settled) = macos::settle(&job);
        job = settled_job;
        self.macos_settled(change, github, &job, settled, &s, next)
            .await?;
        if job.result.is_none() {
            let health = super::super::macos::health(&self.env, &s).await;
            let usage = super::super::macos::usage(&self.env)
                .await
                .unwrap_or_default();
            let (placed_job, placed) = macos::place(&job, &cfg, &health, &usage, at);
            job = placed_job;
            if let Placed::Run { provider, probe } = placed {
                if probe {
                    super::super::macos::report(&self.env, &s, provider, "probing").await;
                }
                if provider == Provider::Github {
                    github.queue(change.number, &job.sha, IOS);
                }
            }
        }
        let rows: Vec<_> = job
            .attempts
            .iter()
            .filter(|a| !before.attempts.contains(a))
            .map(|a| macos::record(&job, a))
            .collect();
        if !rows.is_empty()
            && let Err(e) = platform::d1(&self.env, &rows).await
        {
            worker::console_warn!("macos_jobs: {e}");
        }
        self.state.storage().put(MACOS, &Some(job)).await
    }

    /// Acts on what became of the head's macOS attempt.
    async fn macos_settled(
        &self,
        change: &mut Change,
        github: &mut Actions,
        job: &Job,
        settled: Settled,
        s: &Settings,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        match settled {
            Settled::Pending => {}
            Settled::Ran { provider, passed } => {
                super::super::macos::report(&self.env, s, provider, "ran").await;
                if provider != Provider::Github {
                    let url = job.attempts.last().and_then(|a| a.url.clone());
                    let log = format!(
                        "ios ran on {}: {}\n{}\n",
                        provider.name(),
                        if passed { "passed" } else { "failed" },
                        url.unwrap_or_default()
                    );
                    self.macos_finish(change, job, passed, &log, next).await?;
                }
            }
            Settled::Moved { from, why } => {
                super::super::macos::report(&self.env, s, from, "moved").await;
                if let Some(a) = job.attempts.last() {
                    self.macos_abandon(github, job, a).await;
                }
                let said = format!("!{} ios moved: {why}", change.number);
                let n = Some(change.number);
                let _ =
                    super::super::events::append(&self.env, s, "ci.moved", n, None, &said, false)
                        .await;
            }
            Settled::Failed { why } => {
                let log = format!("ios failed without a result: {why}\n");
                self.macos_finish(change, job, false, &log, next).await?;
            }
        }
        Ok(())
    }

    /// Applies the `ios` check's result, with `log` as its log.
    async fn macos_finish(
        &self,
        change: &mut Change,
        job: &Job,
        ok: bool,
        log: &str,
        next: &mut Vec<Pending>,
    ) -> Result<()> {
        let id = control::job_id(change.number, &job.sha, IOS);
        let outcome = if ok {
            JobOutcome::Passed
        } else {
            JobOutcome::Failed
        };
        self.keep_log(IOS, &id, &Report::of(&id, outcome, log))
            .await?;
        let _ = Self::apply(
            change,
            Event::Finished {
                job: IOS.into(),
                sha: job.sha.clone(),
                ok,
            },
            next,
        );
        Ok(())
    }
}

/// A GitHub attempt whose dispatch failed [`macos::DISPATCH_TRIES`] times is refused, and one
/// dispatched whose run never appeared within `start_within` is unassigned; either is dropped
/// from its dispatch.
fn github_unstarted(github: &mut Actions, job: &Job, start_within: u64, at: u64) -> Job {
    let Some(d) = github
        .dispatches
        .iter()
        .find(|d| d.sha == job.sha && d.checks.iter().any(|c| c == IOS))
    else {
        return job.clone();
    };
    if d.run.is_some() {
        return job.clone();
    }
    let ended = if macos::refused(d.failures) {
        Some((
            Outcome::Refused,
            d.waiting
                .clone()
                .unwrap_or_else(|| "dispatch failed".into()),
        ))
    } else {
        d.dispatched_at
            .map(|t| macos::github(&Value::Null, IOS, t, at, start_within))
            .filter(|o| *o == Outcome::Unassigned)
            .map(|o| (o, "its run never appeared".to_string()))
    };
    match ended {
        Some((o, why)) => {
            github.drop_check(&job.sha, IOS);
            macos::ended_as(job, o, &why, at)
        }
        None => job.clone(),
    }
}
