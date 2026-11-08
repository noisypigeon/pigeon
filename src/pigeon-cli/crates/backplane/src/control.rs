//! The control plane's decisions, apart from the runtime: which change an operation or a push
//! event is for, what a job's Container is told, and what its report means. Shared by both
//! builds, so the Worker's own code only moves bytes; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1 and §4.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::changes::{self, BOUND, Change, Check, Event, Job, MERGE};
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
        /// The mandate it is carried out under, if any.
        mandate: Option<u32>,
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
    /// What `main`'s pipeline last published.
    DeployStatus,
    /// Deploy `main`'s newest push again after it failed.
    DeployRetry,
    /// The repository's git tokens, without their values.
    GitTokens,
    /// Every change, optionally in one state.
    List {
        /// The state.
        state: Option<String>,
    },
    /// A mandate operation, with its arguments; see
    /// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §3.
    Mandate {
        /// The operation.
        operation: String,
        /// Its arguments.
        args: Map<String, Value>,
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
            let mandate = match text(a, "mandate") {
                Some(m) if !m.trim().is_empty() => Some(
                    m.trim()
                        .parse::<u32>()
                        .map_err(|_| format!("`mandate` must be a mandate number, not `{m}`"))?,
                ),
                _ => None,
            };
            Ok(Routed::Propose {
                slug,
                title: need(a, "title")?,
                plan: need(a, "plan")?,
                scope,
                mandate,
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
        "change.merge" => event(Event::Merge { by: by.into() }),
        "ci.rerun" => event(Event::Rerun {
            job: need(a, "job")?,
        }),
        "change.status" => Ok(Routed::Status { number: number(a)? }),
        "deploy.status" => Ok(Routed::DeployStatus),
        "deploy.retry" => Ok(Routed::DeployRetry),
        "git.tokens" => Ok(Routed::GitTokens),
        "approval.wait" => Err("`approval.wait` shows a card, so it is called over MCP".into()),
        "mandate.propose" | "mandate.status" | "mandate.end" => Ok(Routed::Mandate {
            operation: request.operation.clone(),
            args: a.clone(),
        }),
        "change.list" => Ok(Routed::List {
            state: text(a, "state"),
        }),
        other => Err(format!("`{other}` is not a control-plane operation")),
    }
}

/// The fields of a git token `git.tokens` shows; never its value.
pub const TOKEN_FIELDS: [&str; 5] = ["id", "scope", "state", "created_at", "expires_at"];

/// The Artifacts API's token list, `v`, as `git.tokens` shows it: each token's
/// [`TOKEN_FIELDS`] only.
#[must_use]
pub fn token_rows(v: &Value) -> Value {
    let list = v
        .as_array()
        .or_else(|| v["tokens"].as_array())
        .or_else(|| v["items"].as_array());
    Value::Array(
        list.into_iter()
            .flatten()
            .map(|t| {
                TOKEN_FIELDS
                    .iter()
                    .filter_map(|k| t.get(*k).map(|x| ((*k).to_string(), x.clone())))
                    .collect::<Map<String, Value>>()
                    .into()
            })
            .collect(),
    )
}

/// A push, from an Artifacts `cf.artifacts.repo.pushed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    /// The repository, when the event names it.
    pub repo: Option<String>,
    /// What was pushed.
    pub target: Target,
    /// The new head.
    pub sha: String,
}

/// What a push moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A change's branch, by number.
    Change(u32),
    /// `main`: the pipeline publishes what it changed.
    Main,
}

/// What push `event` moved, if it pushed `main` or a change's branch to a commit; a push to any
/// other ref, or a deletion, is none.
#[must_use]
pub fn push(event: &Value) -> Option<Push> {
    let p = event.get("payload").unwrap_or(event);
    let r#ref = p["ref"].as_str()?;
    let sha = p["after"].as_str()?;
    if !changes::is_sha(sha) || sha.bytes().all(|b| b == b'0') {
        return None;
    }
    let target = if r#ref == "refs/heads/main" {
        Target::Main
    } else {
        Target::Change(changes::branch(r#ref)?.0)
    };
    let repo = ["repo", "repoName", "repo_name", "repository"]
        .iter()
        .find_map(|k| p[*k].as_str().or_else(|| p[*k]["name"].as_str()))
        .map(str::to_string);
    Some(Push {
        repo,
        target,
        sha: sha.to_string(),
    })
}

/// What a job's Container is told: where to fetch, what to check out, and what to run.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// For the merge job: who approved the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approver: Option<String>,
    /// For a deploy: the last commit deployed, whose changes since it are published; none
    /// publishes every site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// For a deploy: its credentials, as environment; never logged or shown.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
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
        approver: change.approval.as_ref().map(|a| a.by.clone()),
        since: None,
        env: std::collections::BTreeMap::new(),
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
    /// For the merge job: the commit `main` now holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<String>,
    /// For a deploy: the sites it published.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub published: Vec<String>,
    /// For a deploy: each Worker it deployed, as `<name>: <from> → <to>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<String>,
    /// For a deploy: the `pigeon` it built and stored, if it built one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Build>,
    /// For a deploy: the job images it built and pushed, if their inputs changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Images>,
    /// For the bound job: the head's approval tiers, when `main` has rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<crate::approvals::Tiers>,
}

/// The job images a deploy references, as the `DEPLOY_IMAGES` secret holds them: the commit
/// they were built from, and each container's registry reference by `<worker>/<Class>`; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §4.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Images {
    /// The commit they were built from.
    pub source: String,
    /// Each container's registry reference.
    pub images: std::collections::BTreeMap<String, String>,
}

/// `main`'s `pigeon` built at a commit and stored at `builds/<sha>/pigeon`; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The commit of `main` it was built from.
    pub sha: String,
    /// The binary's SHA-256, hex.
    pub sha256: String,
}

/// The event a finished report is, for `job` on head `sha`; none while it runs.
#[must_use]
pub fn event_of(job: &str, sha: &str, report: &Report) -> Option<Event> {
    if report.outcome == Outcome::Running {
        return None;
    }
    if job == MERGE {
        return Some(match (report.outcome, &report.merged) {
            (Outcome::Passed, Some(commit)) => Event::Merged {
                sha: sha.to_string(),
                commit: Some(commit.clone()),
            },
            _ => Event::MergeFailed {
                sha: sha.to_string(),
                reason: report
                    .violations
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "the merge job failed; see its log".into()),
            },
        });
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
            tiers: report.tiers.clone(),
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

/// The job that publishes what a push to `main` changed.
pub const DEPLOY: &str = "deploy";

/// A deploy job's ID: `main-<sha, 12>-deploy`.
#[must_use]
pub fn deploy_id(sha: &str) -> String {
    format!("main-{}-{DEPLOY}", &sha[..12.min(sha.len())])
}

/// The spec for deploying `sha`, publishing what changed since `since`.
#[must_use]
pub fn deploy_spec(
    remote: &str,
    token: &str,
    sha: &str,
    since: Option<&str>,
    env: std::collections::BTreeMap<String, String>,
) -> JobSpec {
    JobSpec {
        id: deploy_id(sha),
        remote: remote.to_string(),
        token: token.to_string(),
        base: "main".into(),
        branch: "main".into(),
        sha: sha.to_string(),
        number: 0,
        title: String::new(),
        plan: String::new(),
        scope: Vec::new(),
        job: DEPLOY.into(),
        argv: Vec::new(),
        approver: None,
        since: since.map(str::to_string),
        env,
    }
}

/// One deploy, as `deploy.status` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployed {
    /// The commit of `main`.
    pub sha: String,
    /// How it went.
    pub outcome: Outcome,
    /// When it finished, in Unix seconds.
    pub at: u64,
    /// The sites it published.
    #[serde(default)]
    pub published: Vec<String>,
    /// Each Worker it deployed, as `<name>: <from> → <to>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workers: Vec<String>,
    /// Its violations, or why it failed.
    #[serde(default)]
    pub violations: Vec<String>,
}

/// `main`'s pipeline: at most one deploy at a time, always of the newest push, and only after
/// the last one ended; the `MainRun` Durable Object keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    /// The last commit deployed without a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed: Option<String>,
    /// The newest push to `main`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The deploy in progress, and its attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<(String, u32)>,
    /// The last deploys, newest first.
    #[serde(default)]
    pub history: Vec<Deployed>,
    /// The newest `pigeon` a deploy built, which job Containers run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_build: Option<Build>,
    /// The newest job images, which deploys reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Images>,
    /// The `DEPLOY_IMAGES` secret as last seen; a different one is the owner's push, and wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images_seed: Option<Images>,
}

/// How many deploys `deploy.status` keeps.
pub const HISTORY: usize = 20;

impl Pipeline {
    /// `main` moved to `sha`.
    pub fn pushed(&mut self, sha: &str) {
        if changes::is_sha(sha) {
            self.target = Some(sha.to_string());
        }
    }

    /// The deploy to start now, if none runs and the newest push isn't deployed.
    pub fn start(&mut self) -> Option<String> {
        if self.running.is_some() {
            return None;
        }
        let target = self.target.clone()?;
        if self.deployed.as_deref() == Some(target.as_str()) {
            return None;
        }
        self.running = Some((target.clone(), 1));
        Some(target)
    }

    /// The images a deploy is to reference, given the `DEPLOY_IMAGES` secret: the secret when
    /// it changed since last seen, as after the owner's push, else the newest a deploy built.
    pub fn images_for_deploy(&mut self, secret: Option<Images>) -> Option<Images> {
        if secret.is_some() && secret != self.images_seed {
            self.images.clone_from(&secret);
            self.images_seed = secret;
        }
        self.images.clone().or_else(|| self.images_seed.clone())
    }

    /// The pipeline as `deploy.status` shows it: images by their commit only, since their
    /// references name the account.
    #[must_use]
    pub fn shown(&self) -> Self {
        let hide = |i: &Option<Images>| {
            i.as_ref().map(|i| Images {
                source: i.source.clone(),
                images: i
                    .images
                    .keys()
                    .map(|k| (k.clone(), "…".to_string()))
                    .collect(),
            })
        };
        Self {
            images: hide(&self.images),
            images_seed: hide(&self.images_seed),
            ..self.clone()
        }
    }

    /// The running deploy's report arrived, at `at`.
    pub fn finished(&mut self, report: &Report, at: u64) {
        let Some((sha, _)) = self.running.take() else {
            return;
        };
        if report.outcome == Outcome::Passed {
            self.deployed = Some(sha.clone());
        }
        // A build stored from `main` is good for jobs whatever else the deploy did.
        if let Some(b) = report
            .build
            .clone()
            .filter(|b| b.sha == sha && b.sha256.len() == 64)
        {
            self.latest_build = Some(b);
        }
        // Images pushed from `main` are good for deploys whatever else the deploy did.
        if let Some(i) = report.images.clone().filter(|i| i.source == sha) {
            self.images = Some(i);
        }
        // A failure waits for the next push, which publishes everything since the last deploy.
        if self.target.as_deref() == Some(sha.as_str()) {
            self.target = None;
        }
        self.record(Deployed {
            sha,
            outcome: report.outcome,
            at,
            published: report.published.clone(),
            workers: report.workers.clone(),
            violations: report.violations.clone(),
        });
    }

    /// Deploy the newest push again: after it failed, as when its image was missing, from the
    /// last deploy that passed; or after it passed, everything, as when new images or a change
    /// to rendering take effect only on a deploy. None while a deploy runs or waits.
    pub fn retry(&mut self) -> Option<String> {
        if self.running.is_some() || self.target.is_some() {
            return None;
        }
        let last = self.history.first()?.sha.clone();
        if self.deployed.as_deref() == Some(last.as_str()) {
            // Nothing changed since itself, so only a deploy of everything redeploys its Workers.
            self.deployed = None;
        }
        self.target = Some(last);
        self.start()
    }

    /// The running deploy's Container was lost: the same deploy, again, until its attempts are
    /// used.
    pub fn lost(&mut self, at: u64) -> Option<String> {
        let (sha, attempts) = self.running.take()?;
        if attempts < crate::runs::MAX_ATTEMPTS {
            self.running = Some((sha.clone(), attempts + 1));
            return Some(sha);
        }
        if self.target.as_deref() == Some(sha.as_str()) {
            self.target = None;
        }
        self.record(Deployed {
            sha,
            outcome: Outcome::Failed,
            at,
            published: Vec::new(),
            workers: Vec::new(),
            violations: vec![format!(
                "the deploy's Container was lost {} times",
                crate::runs::MAX_ATTEMPTS
            )],
        });
        None
    }

    fn record(&mut self, d: Deployed) {
        self.history.insert(0, d);
        self.history.truncate(HISTORY);
    }
}

/// Whether a job's start failed only for want of a Container, which is no fault of the job:
/// it waits for a free one, and uses no attempt.
#[must_use]
pub fn no_capacity(why: &str) -> bool {
    why.contains("Maximum number of running container instances")
        || why.contains("no container instance that can be provided")
}

/// The Container class a job runs in (approvals-images-state ADR §4): Rust checks and builds
/// need `standard-4`; the bound job, merges and deploys only run git and small tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// `standard-4`: the change's checks.
    Checker,
    /// `standard-1`: the bound job, merges and deploys.
    Runner,
}

impl Class {
    /// The Durable Object binding of the class's namespace.
    #[must_use]
    pub fn binding(self) -> &'static str {
        match self {
            Self::Checker => "CHECKER",
            Self::Runner => "RUNNER",
        }
    }
}

/// The class `job` runs in.
#[must_use]
pub fn class_of(job: &str) -> Class {
    // A deploy builds the Worker to wasm, so it runs on `Checker`; see
    // .claude/decisions/2026-11-07-development-control-plane-deploys-itself.md.
    if job == BOUND || job == MERGE {
        Class::Runner
    } else {
        Class::Checker
    }
}

/// The most jobs of one change that run at once, so one large change can't take the fleet.
pub const MAX_RUNNING: usize = 6;

/// Whether a change with `running` jobs may start another.
#[must_use]
pub fn may_start(running: usize) -> bool {
    running < MAX_RUNNING
}
