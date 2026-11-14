//! One macOS job across providers: its attempts, settling an attempt's outcome, placing it on
//! the next provider, and its `macos_jobs` rows; pure. See
//! `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §2.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Config, GIVE_UP, Health, Kind, Next, Outcome, Provider, Usage, Verdict, WAIT_RETRY, minutes,
    next, refused, verdict,
};
use crate::control::Statement;

/// One attempt on a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// The provider.
    pub provider: Provider,
    /// Its number in the job, from 1.
    pub n: u32,
    /// The provider's run or build, once dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// When it was placed.
    pub queued_at: u64,
    /// When a machine took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// When it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// How it stands.
    pub outcome: Outcome,
    /// Dispatches that failed in a row.
    #[serde(default)]
    pub failures: u32,
    /// It probes a down provider.
    #[serde(default)]
    pub probe: bool,
    /// Why, when it ended without running, or its dispatch last failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A macOS job: a change head's `ios` check, or a release's archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// What it is.
    pub kind: Kind,
    /// The change, or the release's build number.
    pub number: u64,
    /// The commit.
    pub sha: String,
    /// When it began.
    pub since: u64,
    /// Its attempts, oldest first.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    /// The first attempt of the current round; a wait starts a new round from the order's start.
    #[serde(default)]
    pub round: usize,
    /// Why it waits for a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<String>,
    /// Nothing is placed before this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_at: Option<u64>,
    /// The provider the owner chose, ignoring health and minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced: Option<Provider>,
    /// Providers that count as tried in the first round: GitHub while the CI window is open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<Provider>,
    /// Its result, once it ran or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Ended>,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    /// It ran and passed.
    pub passed: bool,
    /// The provider it ran on, if it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// Why it failed without running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// What became of the current attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Nothing yet, or there is no attempt in flight.
    Pending,
    /// The job ran to a result on `provider`.
    Ran {
        /// The provider.
        provider: Provider,
        /// It passed.
        passed: bool,
    },
    /// The job moves away from `from`.
    Moved {
        /// The provider it leaves.
        from: Provider,
        /// Why.
        why: String,
    },
    /// The job fails without a result.
    Failed {
        /// Why.
        why: String,
    },
}

/// Where the job was placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// A new attempt on `provider`, to dispatch.
    Run {
        /// The provider.
        provider: Provider,
        /// It probes a down provider.
        probe: bool,
    },
    /// No provider; offered again at `next_at`.
    Wait {
        /// Why, `macOS (…)`.
        why: String,
    },
    /// A release's archive waited [`GIVE_UP`] and fails.
    GaveUp {
        /// Why.
        why: String,
    },
    /// Nothing to place: an attempt is in flight, it ended, or its wait isn't over.
    Nothing,
}

impl Job {
    /// A new job of `kind` for `number` at `sha`; `forced` runs it there alone, and `skip` counts
    /// as tried in its first round.
    #[must_use]
    pub fn new(
        kind: Kind,
        number: u64,
        sha: &str,
        now: u64,
        forced: Option<Provider>,
        skip: Vec<Provider>,
    ) -> Self {
        Self {
            kind,
            number,
            sha: sha.into(),
            since: now,
            attempts: Vec::new(),
            round: 0,
            waiting: None,
            next_at: None,
            forced,
            skip,
            result: None,
        }
    }

    /// The attempt in flight.
    #[must_use]
    pub fn current(&self) -> Option<&Attempt> {
        self.attempts.last().filter(|a| !a.outcome.ended())
    }

    /// The providers tried this round.
    #[must_use]
    pub fn tried(&self) -> Vec<Provider> {
        let mut t: Vec<Provider> = self.attempts[self.round.min(self.attempts.len())..]
            .iter()
            .map(|a| a.provider)
            .collect();
        if self.round == 0 {
            t.extend(self.skip.iter().copied());
        }
        t
    }

    /// Whether an earlier attempt lost its machine.
    #[must_use]
    pub fn lost_before(&self) -> bool {
        let n = self.attempts.len().saturating_sub(1);
        self.attempts[..n]
            .iter()
            .any(|a| a.outcome == Outcome::Lost)
    }

    /// The provider it ran on, or the last it was tried on.
    #[must_use]
    pub fn provider(&self) -> Option<Provider> {
        self.attempts.last().map(|a| a.provider)
    }
}

fn with_current(job: &Job, f: impl FnOnce(&mut Attempt)) -> Job {
    let mut j = job.clone();
    if let Some(a) = j.attempts.last_mut().filter(|a| !a.outcome.ended()) {
        f(a);
    }
    j
}

/// `job` once its current attempt was dispatched as `id`, shown at `url`.
#[must_use]
pub fn dispatched(job: &Job, id: &str, url: Option<&str>) -> Job {
    with_current(job, |a| {
        a.id = Some(id.into());
        a.url = url.map(str::to_string);
        a.failures = 0;
        a.why = None;
    })
}

/// `job` once its current attempt's dispatch failed at `now`; the [`super::DISPATCH_TRIES`]th
/// in a row is `refused`.
#[must_use]
pub fn dispatch_failed(job: &Job, why: &str, now: u64) -> Job {
    with_current(job, |a| {
        a.failures += 1;
        a.why = Some(why.into());
        if refused(a.failures) {
            a.outcome = Outcome::Refused;
            a.finished_at = Some(now);
        }
    })
}

/// `job` once the provider's answer put its current attempt at `outcome`, at `now`.
#[must_use]
pub fn observed(job: &Job, outcome: Outcome, now: u64) -> Job {
    with_current(job, |a| {
        if a.started_at.is_none() && (outcome == Outcome::Running || outcome.ran()) {
            a.started_at = Some(now);
        }
        if outcome.ended() {
            a.finished_at = Some(now);
        }
        a.outcome = outcome;
    })
}

/// `job` with its current attempt ended as `outcome` for `why`, as when a provider's config
/// differs from `main`'s.
#[must_use]
pub fn ended_as(job: &Job, outcome: Outcome, why: &str, now: u64) -> Job {
    with_current(job, |a| {
        a.outcome = outcome;
        a.finished_at = Some(now);
        a.why = Some(why.into());
    })
}

/// The verdict on `job`'s newest attempt, once it ended, and the job with its result when that
/// ends it.
#[must_use]
pub fn settle(job: &Job) -> (Job, Settled) {
    let Some(a) = job.attempts.last().filter(|_| job.result.is_none()) else {
        return (job.clone(), Settled::Pending);
    };
    if !a.outcome.ended() {
        return (job.clone(), Settled::Pending);
    }
    let mut j = job.clone();
    let settled = match verdict(a.provider, a.outcome, job.lost_before()) {
        Verdict::Wait => Settled::Pending,
        Verdict::Done { passed } => {
            j.result = Some(Ended {
                passed,
                provider: Some(a.provider),
                why: None,
            });
            Settled::Ran {
                provider: a.provider,
                passed,
            }
        }
        Verdict::Move { why } => {
            let why = a
                .why
                .as_ref()
                .map_or(why.clone(), |w| format!("{why}: {w}"));
            if job.forced.is_some() {
                j.result = Some(Ended {
                    passed: false,
                    provider: None,
                    why: Some(why.clone()),
                });
                Settled::Failed { why }
            } else {
                Settled::Moved {
                    from: a.provider,
                    why,
                }
            }
        }
        Verdict::Fail { why } => {
            j.result = Some(Ended {
                passed: false,
                provider: None,
                why: Some(why.clone()),
            });
            Settled::Failed { why }
        }
    };
    (j, settled)
}

/// Places `job` when nothing is in flight: the forced provider, or [`next`]'s, or a wait; a
/// release's archive gives up after [`GIVE_UP`].
#[must_use]
pub fn place(job: &Job, cfg: &Config, health: &Health, usage: &Usage, now: u64) -> (Job, Placed) {
    let ended = job.attempts.last().is_none_or(|a| a.outcome.ended());
    if job.result.is_some() || !ended || job.next_at.is_some_and(|t| now < t) {
        return (job.clone(), Placed::Nothing);
    }
    let mut j = job.clone();
    if j.next_at.take().is_some() {
        j.round = j.attempts.len();
    }
    let chosen = match job.forced {
        Some(p) if j.attempts.is_empty() => Next::Run {
            provider: p,
            probe: false,
        },
        Some(_) => return (j, Placed::Nothing),
        None => next(cfg, health, usage, &j.tried(), now),
    };
    match chosen {
        Next::Run { provider, probe } => {
            j.waiting = None;
            j.attempts.push(Attempt {
                provider,
                n: u32::try_from(j.attempts.len() + 1).unwrap_or(u32::MAX),
                id: None,
                url: None,
                queued_at: now,
                started_at: None,
                finished_at: None,
                outcome: Outcome::Waiting,
                failures: 0,
                probe,
                why: None,
            });
            (j, Placed::Run { provider, probe })
        }
        Next::Wait { why } if job.kind == Kind::Archive && now >= job.since + GIVE_UP => {
            let why = format!("no provider took the archive in 2 hours: {why}");
            j.result = Some(Ended {
                passed: false,
                provider: None,
                why: Some(why.clone()),
            });
            (j, Placed::GaveUp { why })
        }
        Next::Wait { why } => {
            j.waiting = Some(why.clone());
            j.next_at = Some(now + WAIT_RETRY);
            (j, Placed::Wait { why })
        }
    }
}

/// What `change_status` and `ios.status` show of `job`.
#[must_use]
pub fn view(job: &Job) -> Value {
    json!({
        "kind": job.kind.name(),
        "sha": job.sha,
        "waiting": job.waiting.as_ref().map(|w| format!("waiting: {w}")),
        "result": job.result,
        "attempts": job.attempts.iter().map(|a| json!({
            "provider": a.provider.name(),
            "attempt": a.n,
            "outcome": a.outcome.name(),
            "url": a.url,
            "why": a.why,
        })).collect::<Vec<_>>(),
    })
}

/// Writes `job`'s attempt `a` to `macos_jobs`, replacing its earlier row.
#[must_use]
pub fn record(job: &Job, a: &Attempt) -> Statement {
    let mins = match (a.started_at, a.finished_at) {
        (Some(s), Some(f)) => Some(minutes(s, f)),
        _ => None,
    };
    Statement {
        sql: "INSERT INTO macos_jobs (kind, number, sha, attempt, provider, run, url, outcome, \
              queued_at, started_at, finished_at, minutes) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
              ON CONFLICT (kind, number, sha, attempt) DO UPDATE SET run = ?6, url = ?7, \
              outcome = ?8, started_at = ?10, finished_at = ?11, minutes = ?12",
        params: vec![
            json!(job.kind.name()),
            json!(job.number),
            json!(job.sha),
            json!(a.n),
            json!(a.provider.name()),
            json!(a.id),
            json!(a.url),
            json!(a.outcome.name()),
            json!(a.queued_at),
            json!(a.started_at),
            json!(a.finished_at),
            json!(mins),
        ],
    }
}

/// Each provider's minutes since `from`.
#[must_use]
pub fn usage_since(from: u64) -> Statement {
    Statement {
        sql: "SELECT provider, COALESCE(SUM(minutes), 0) AS minutes FROM macos_jobs \
              WHERE queued_at >= ?1 GROUP BY provider",
        params: vec![json!(from)],
    }
}

/// [`Usage`] from [`usage_since`]'s rows.
#[must_use]
pub fn usage_of(rows: &[Value]) -> Usage {
    rows.iter()
        .filter_map(|r| {
            Some((
                Provider::parse(r["provider"].as_str()?)?,
                r["minutes"].as_u64()?,
            ))
        })
        .collect()
}

/// The attempts that ended in `[from, from + 3600)`, for insights.
#[must_use]
pub fn ended_in(from: u64) -> Statement {
    Statement {
        sql: "SELECT kind, provider, outcome, minutes FROM macos_jobs \
              WHERE finished_at >= ?1 AND finished_at < ?2",
        params: vec![json!(from), json!(from + 3600)],
    }
}

/// The attempts moved away from since `from`, newest first, for the digest.
#[must_use]
pub fn moved_since(from: u64) -> Statement {
    Statement {
        sql: "SELECT kind, number, attempt, provider, outcome FROM macos_jobs \
              WHERE finished_at >= ?1 AND outcome IN ('unassigned', 'lost', 'refused') \
              ORDER BY finished_at DESC LIMIT 20",
        params: vec![json!(from)],
    }
}
