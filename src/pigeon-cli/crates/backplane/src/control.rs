//! The control plane's decisions, apart from the runtime: which change an operation or a push
//! event is for, what a job's Container is told, and what its report means. Shared by both
//! builds, so the Worker's own code only moves bytes; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1 and §4.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::changes::{self, BOUND, Change, Check, Event, Job};
use crate::wire::Request;

/// A request for the control plane, as the Worker routes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// Create change `slug`, numbered by the Worker.
    Propose {
        /// Its slug.
        slug: String,
        /// Its title.
        title: String,
        /// Its plan.
        plan: String,
        /// Its scope, one glob each.
        scope: Vec<String>,
    },
    /// Apply an event to change `number`; `by` is filled in from Access.
    Event {
        /// The change.
        number: u32,
        /// What happened.
        event: Event,
    },
    /// Change `number`'s status.
    Status {
        /// The change.
        number: u32,
    },
    /// Every change, optionally in one state.
    List {
        /// The state.
        state: Option<String>,
    },
}

fn text(args: &Map<String, Value>, k: &str) -> Option<String> {
    match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn need(args: &Map<String, Value>, k: &str) -> Result<String, String> {
    text(args, k)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("argument `{k}` is empty"))
}

fn number(args: &Map<String, Value>) -> Result<u32, String> {
    need(args, "number")?
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "argument `number` must be a change's number".into())
}

fn slug_ok(slug: &str) -> bool {
    changes::branch(&format!("change/1-{slug}")).is_some()
}

/// Routes a validated control-plane request, as `by` at `at`.
///
/// # Errors
///
/// A message for an argument the catalog's types allow but the change rules don't.
pub fn route(request: &Request, by: &str, at: u64) -> Result<Routed, String> {
    let a = &request.args;
    let event = |event| {
        Ok(Routed::Event {
            number: number(a)?,
            event,
        })
    };
    match request.operation.as_str() {
        "change.propose" => {
            let slug = need(a, "slug")?;
            if !slug_ok(&slug) {
                return Err(format!(
                    "slug `{slug}` must be lowercase words joined by `-`"
                ));
            }
            let scope: Vec<String> = need(a, "scope")?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            Ok(Routed::Propose {
                slug,
                title: need(a, "title")?,
                plan: need(a, "plan")?,
                scope,
            })
        }
        "change.authorize" => event(Event::ApprovePlan { by: by.into(), at }),
        "change.pushed" => event(Event::Pushed {
            sha: need(a, "sha")?,
        }),
        "change.approve" => event(Event::Approve {
            by: by.into(),
            sha: need(a, "sha")?,
            at,
        }),
        "change.close" => event(Event::Close),
        "ci.rerun" => event(Event::Rerun {
            job: need(a, "job")?,
        }),
        "change.status" => Ok(Routed::Status { number: number(a)? }),
        "change.list" => Ok(Routed::List {
            state: text(a, "state"),
        }),
        other => Err(format!("`{other}` is not a control-plane operation")),
    }
}

/// A push, from an Artifacts `cf.artifacts.repo.pushed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    /// The repository, when the event names it.
    pub repo: Option<String>,
    /// The change the branch is for.
    pub number: u32,
    /// The new head.
    pub sha: String,
}

/// The change push `event` is for, if it pushed a change's branch to a commit; a push to any
/// other ref, or a deletion, is none.
#[must_use]
pub fn push(event: &Value) -> Option<Push> {
    let p = event.get("payload").unwrap_or(event);
    let r#ref = p["ref"].as_str()?;
    let sha = p["after"].as_str()?;
    if !changes::is_sha(sha) || sha.bytes().all(|b| b == b'0') {
        return None;
    }
    let (number, _) = changes::branch(r#ref)?;
    let repo = ["repo", "repoName", "repo_name", "repository"]
        .iter()
        .find_map(|k| p[*k].as_str().or_else(|| p[*k]["name"].as_str()))
        .map(str::to_string);
    Some(Push {
        repo,
        number,
        sha: sha.to_string(),
    })
}

/// What a job's Container is told: where to fetch, what to check out, and what to run.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSpec {
    /// Its ID, unique per change, head and job.
    pub id: String,
    /// The repository's git remote.
    pub remote: String,
    /// A read token for it, minted for this job; never logged.
    pub token: String,
    /// The default branch the change is based on.
    pub base: String,
    /// The change's branch.
    pub branch: String,
    /// The head to check out; the bound job refuses one that isn't the branch's head.
    pub sha: String,
    /// The change's number.
    pub number: u32,
    /// Its title, for the pull-request policy.
    pub title: String,
    /// Its plan, as the policy's body.
    pub plan: String,
    /// Its approved scope.
    pub scope: Vec<String>,
    /// The job.
    pub job: String,
    /// Its command; empty for the bound job.
    #[serde(default)]
    pub argv: Vec<String>,
}

/// Shows no token and no remote.
impl std::fmt::Debug for JobSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobSpec")
            .field("id", &self.id)
            .field("job", &self.job)
            .field("sha", &self.sha)
            .finish_non_exhaustive()
    }
}

/// A job's ID: `<number>-<sha, 12>-<job>`, safe as a Durable Object name and an R2 key part.
#[must_use]
pub fn job_id(number: u32, sha: &str, job: &str) -> String {
    let job: String = job
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("{number}-{}-{job}", &sha[..12.min(sha.len())])
}

/// The spec for `job` on `change`'s head.
///
/// # Errors
///
/// A message when the change has no head or no such job.
pub fn spec(change: &Change, job: &str, remote: &str, token: &str) -> Result<JobSpec, String> {
    let sha = change.head.clone().ok_or("the change has no head")?;
    let j: &Job = change
        .jobs
        .iter()
        .find(|j| j.name == job)
        .ok_or_else(|| format!("no job `{job}`"))?;
    Ok(JobSpec {
        id: job_id(change.number, &sha, job),
        remote: remote.to_string(),
        token: token.to_string(),
        base: "main".into(),
        branch: change.branch(),
        sha,
        number: change.number,
        title: change.title.clone(),
        plan: change.plan.clone(),
        scope: change.scope.clone(),
        job: job.to_string(),
        argv: j.argv.clone(),
    })
}

/// Where a job is, as its Container reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Still running.
    Running,
    /// It passed.
    Passed,
    /// It failed.
    Failed,
}

/// A job's report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Its ID.
    pub id: String,
    /// Where it is.
    pub outcome: Outcome,
    /// The last of its output, scrubbed of its token.
    #[serde(default)]
    pub log: String,
    /// For the bound job: every violation.
    #[serde(default)]
    pub violations: Vec<String>,
    /// For the bound job: the checks `change.scope` named.
    #[serde(default)]
    pub checks: Vec<Check>,
}

/// The event a finished report is, for `job` on head `sha`; none while it runs.
#[must_use]
pub fn event_of(job: &str, sha: &str, report: &Report) -> Option<Event> {
    if report.outcome == Outcome::Running {
        return None;
    }
    if job == BOUND {
        let mut violations = report.violations.clone();
        if report.outcome == Outcome::Failed && violations.is_empty() {
            violations.push("the bound job failed; see its log".into());
        }
        return Some(Event::Bounded {
            sha: sha.to_string(),
            violations,
            checks: report.checks.clone(),
        });
    }
    Some(Event::Finished {
        job: job.to_string(),
        sha: sha.to_string(),
        ok: report.outcome == Outcome::Passed,
    })
}

/// The most of a job's log kept: its last 64 KiB.
pub const LOG_TAIL: usize = 64 * 1024;

/// The last [`LOG_TAIL`] bytes of `log`, cut at a character boundary.
#[must_use]
pub fn tail(log: &str) -> &str {
    last(log, LOG_TAIL)
}

/// The last `n` bytes of `log`, or fewer, cut at a character boundary.
#[must_use]
pub fn last(log: &str, n: usize) -> &str {
    if log.len() <= n {
        return log;
    }
    let mut at = log.len() - n;
    while !log.is_char_boundary(at) {
        at += 1;
    }
    &log[at..]
}

/// One SQL statement and its parameters, in order, for D1's `bind`.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    /// The SQL, with `?1`… parameters.
    pub sql: &'static str,
    /// The parameters: strings, numbers or null.
    pub params: Vec<Value>,
}

/// Ensures the instance row a change's foreign key needs.
#[must_use]
pub fn instance_row(instance: &str, now: u64) -> Statement {
    Statement {
        sql: "INSERT OR IGNORE INTO instances (id, name, created) VALUES (?1, ?1, ?2)",
        params: vec![instance.into(), now.into()],
    }
}

/// Mirrors `change` to D1's `changes` and its head's `jobs`, at `now`.
#[must_use]
pub fn mirror(
    instance: &str,
    repo: &str,
    change: &Change,
    logs: &Map<String, Value>,
    now: u64,
) -> Vec<Statement> {
    let opt = |v: Option<&str>| v.map_or(Value::Null, Into::into);
    let mut out = vec![instance_row(instance, now)];
    out.push(Statement {
        sql:
            "INSERT INTO changes (instance, repo, number, slug, title, plan, scope, proposer, state,
                head, plan_approver, approver, approved_sha, violations, created, updated)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
              ON CONFLICT (instance, repo, number) DO UPDATE SET
                state = excluded.state, head = excluded.head,
                plan_approver = excluded.plan_approver, approver = excluded.approver,
                approved_sha = excluded.approved_sha, violations = excluded.violations,
                updated = excluded.updated",
        params: vec![
            instance.into(),
            repo.into(),
            change.number.into(),
            change.slug.clone().into(),
            change.title.clone().into(),
            change.plan.clone().into(),
            Value::String(serde_json::to_string(&change.scope).unwrap_or_default()),
            change.proposer.clone().into(),
            change.state.name().into(),
            opt(change.head.as_deref()),
            opt(change.plan_approval.as_ref().map(|a| a.by.as_str())),
            opt(change.approval.as_ref().map(|a| a.by.as_str())),
            opt(change.approval.as_ref().and_then(|a| a.sha.as_deref())),
            Value::String(serde_json::to_string(&change.violations).unwrap_or_default()),
            now.into(),
        ],
    });
    if let Some(sha) = &change.head {
        for j in &change.jobs {
            let status = serde_json::to_value(j.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            out.push(Statement {
                sql: "INSERT INTO jobs (instance, repo, change, sha, name, status, attempts, log, updated)
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                      ON CONFLICT (instance, repo, change, sha, name) DO UPDATE SET
                        status = excluded.status, attempts = excluded.attempts,
                        log = COALESCE(excluded.log, jobs.log), updated = excluded.updated",
                params: vec![
                    instance.into(),
                    repo.into(),
                    change.number.into(),
                    sha.clone().into(),
                    j.name.clone().into(),
                    status.into(),
                    j.attempts.into(),
                    logs.get(&j.name).cloned().unwrap_or(Value::Null),
                    now.into(),
                ],
            });
        }
    }
    out
}

/// The next change number: one past the highest, and never below `floor`.
#[must_use]
pub fn next_number(instance: &str, repo: &str, floor: u32) -> Statement {
    Statement {
        sql: "SELECT MAX(COALESCE(MAX(number), 0) + 1, ?3) AS n FROM changes
              WHERE instance = ?1 AND repo = ?2",
        params: vec![instance.into(), repo.into(), floor.into()],
    }
}

/// Every change, newest first, optionally in one state.
#[must_use]
pub fn list(instance: &str, repo: &str, state: Option<&str>) -> Statement {
    Statement {
        sql: "SELECT number, slug, title, state, head, updated FROM changes
              WHERE instance = ?1 AND repo = ?2 AND (?3 IS NULL OR state = ?3)
              ORDER BY number DESC LIMIT 100",
        params: vec![
            instance.into(),
            repo.into(),
            state.map_or(Value::Null, Into::into),
        ],
    }
}

/// An audit row: who ran what, on what, and how it went.
#[must_use]
pub fn audit(
    at: u64,
    actor: &str,
    operation: &str,
    instance: &str,
    target: Option<&str>,
    outcome: &str,
) -> Statement {
    Statement {
        sql: "INSERT INTO audit (at, actor, operation, instance, target, outcome)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params: vec![
            at.into(),
            actor.into(),
            operation.into(),
            instance.into(),
            target.map_or(Value::Null, Into::into),
            outcome.into(),
        ],
    }
}
