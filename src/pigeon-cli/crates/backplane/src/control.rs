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
    /// A platform run's operation; see
    /// .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 4's amendment.
    Platform {
        /// `platform.plan`, `platform.apply` or `platform.status`.
        operation: String,
        /// Its arguments.
        args: Map<String, Value>,
    },
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
        /// The session proposing it.
        session: Option<String>,
        /// For a design change: the globs its ADR will govern.
        affects: Vec<String>,
        /// Whether its merge gate runs the full test suite whatever its scope.
        full: bool,
        /// The evicted change it resumes, if any.
        resumes: Option<u32>,
        /// The open change it is built on, if any.
        after: Option<u32>,
        /// What it asks the owner at its plan approval.
        asks: Vec<crate::core::change::asks::Ask>,
    },
    /// Apply an event to change `number`; `by` is filled in from Access.
    Event {
        /// The change.
        number: u32,
        /// What happened.
        event: Event,
    },
    /// The owner's break-glass on change `number`: its head's code checks on the `Checker`.
    BreakGlass {
        /// The change.
        number: u32,
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
    /// `change.evictable`: each open change as an eviction candidate.
    Evictable,
    /// `change.note`: a note on a change, for the session that follows it.
    Note {
        /// The change.
        number: u32,
        /// The note.
        text: String,
    },
    /// `change.handoff`: a session offers its change to another.
    Handoff {
        /// The change.
        number: u32,
        /// The session offered it.
        session: String,
        /// What to know.
        note: String,
    },
    /// `change.yield`: a session parks its change behind another, or ends the yield.
    Yield {
        /// The change.
        number: u32,
        /// The change it waits for; none with `off`.
        to: Option<u32>,
        /// Why.
        reason: String,
        /// End the yield instead.
        off: bool,
    },
    /// `change.pin`: the owner pins a change against eviction, or unpins it.
    Pin {
        /// The change.
        number: u32,
        /// Unpin instead.
        off: bool,
    },
    /// `digest.show`: the owner's digest now.
    Digest,
    /// `apple.check`: the Apple connection's health.
    AppleCheck,
    /// `ios.release`: start a release of `main`'s mirrored head.
    IosRelease,
    /// `ios.status`: the iOS releases.
    IosStatus,
    /// `session.status`, with its arguments.
    SessionStatus {
        /// Its arguments.
        args: serde_json::Map<String, Value>,
    },
    /// `session.end`, with its arguments.
    SessionEnd {
        /// Its arguments.
        args: serde_json::Map<String, Value>,
    },
    /// `session.start`, with its arguments.
    SessionStart {
        /// Its arguments.
        args: serde_json::Map<String, Value>,
    },
    /// `events.wait`, with its arguments; see
    /// `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §2.
    Events {
        /// Its arguments.
        args: serde_json::Map<String, Value>,
    },
    /// A task operation, with its arguments; see
    /// `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.
    Task {
        /// The operation.
        operation: String,
        /// Its arguments.
        args: serde_json::Map<String, Value>,
    },
    /// A `grant.*` operation on grant `name` (empty for `grant.list`); see
    /// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.
    Grant {
        /// The operation.
        operation: String,
        /// The grant.
        name: String,
    },
    /// `ci.window` for `hours`, 0 closing it; see
    /// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md`, its 2026-11-11
    /// amendment.
    CiWindow {
        /// How long it lasts.
        hours: u32,
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

/// Whether argument `k` is `true`, as a boolean or as text.
fn flag(args: &Map<String, Value>, k: &str) -> bool {
    text(args, k).is_some_and(|v| v.trim() == "true")
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

/// `change.authorize`'s event: with the owner's answers when the redeemer added them.
fn authorize(a: &Map<String, Value>, by: &str, at: u64) -> Event {
    let answers = crate::core::change::asks::answers_of(a.get("answers"));
    if answers.is_empty() {
        Event::ApprovePlan { by: by.into(), at }
    } else {
        Event::ApprovePlanAnswered {
            by: by.into(),
            at,
            answers,
        }
    }
}

/// `change.propose`'s arguments as a route.
fn propose_route(a: &Map<String, Value>) -> Result<Routed, String> {
    let slug = need(a, "slug")?;
    if !slug_ok(&slug) {
        return Err(format!(
            "slug `{slug}` must be lowercase words joined by `-`"
        ));
    }
    let globs = |s: &str| -> Vec<String> {
        s.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };
    let scope = globs(&need(a, "scope")?);
    let affects = globs(&text(a, "affects").unwrap_or_default());
    let session = crate::sessions::session(text(a, "session").as_deref())?;
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
        session,
        affects,
        full: flag(a, "full"),
        resumes: number_arg(a, "resumes")?,
        after: number_arg(a, "after")?,
        asks: crate::core::change::asks::parse(&text(a, "asks").unwrap_or_default())?,
    })
}

/// The change number in argument `k`, if given.
fn number_arg(a: &Map<String, Value>, k: &str) -> Result<Option<u32>, String> {
    match text(a, k) {
        Some(r) if !r.trim().is_empty() => r
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("`{k}` must be a change number, not `{r}`")),
        _ => Ok(None),
    }
}

/// A `grant.*` request: `grant.list`, or an operation on the grant `name` names.
fn grant(request: &Request) -> Result<Routed, String> {
    let operation = request.operation.clone();
    if operation == "grant.list" {
        return Ok(Routed::Grant {
            operation,
            name: String::new(),
        });
    }
    let name = need(&request.args, "name")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "`name` must be a grant's name, [a-z0-9-]+, not `{name}`"
        ));
    }
    Ok(Routed::Grant { operation, name })
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
        "change.propose" => propose_route(a),
        "change.note" => Ok(Routed::Note {
            number: number(a)?,
            text: need(a, "text")?,
        }),
        "change.handoff" => Ok(Routed::Handoff {
            number: number(a)?,
            session: crate::sessions::session(Some(&need(a, "session")?))?
                .ok_or("`session` names the session to offer it to")?,
            note: text(a, "note").unwrap_or_default(),
        }),
        "change.yield" => {
            let off = flag(a, "off");
            let to = match text(a, "to") {
                Some(t) if !t.trim().is_empty() => Some(
                    t.trim()
                        .parse::<u32>()
                        .map_err(|_| format!("`to` must be a change number, not `{t}`"))?,
                ),
                _ if off => None,
                _ => return Err("`to` names the change to yield to".into()),
            };
            Ok(Routed::Yield {
                number: number(a)?,
                to,
                reason: text(a, "reason").unwrap_or_default(),
                off,
            })
        }
        "change.pin" => Ok(Routed::Pin {
            number: number(a)?,
            off: flag(a, "off"),
        }),
        "change.authorize" => event(authorize(a, by, at)),
        "change.pushed" => event(Event::Pushed {
            sha: need(a, "sha")?,
        }),
        "change.approve" => event(Event::Approve {
            by: by.into(),
            sha: need(a, "sha")?,
            at,
        }),
        "change.close" => event(Event::Close),
        "change.merge" if flag(a, "full") => event(Event::MergeFull { by: by.into() }),
        "change.merge" => event(Event::Merge { by: by.into() }),
        "ci.rerun" => match text(a, "on").as_deref() {
            None | Some("" | "github") => event(Event::Rerun {
                job: need(a, "job")?,
            }),
            Some("cloudflare") => Err(format!("`on: cloudflare` is the owner's `{BREAK_GLASS}`")),
            Some(other) => Err(format!("`on` is `github` or `cloudflare`, not `{other}`")),
        },
        BREAK_GLASS => Ok(Routed::BreakGlass { number: number(a)? }),
        crate::ci_window::OPERATION => Ok(Routed::CiWindow {
            hours: crate::ci_window::hours(text(a, "hours").as_deref())?,
        }),
        "change.status" => Ok(Routed::Status { number: number(a)? }),
        "deploy.status" => Ok(Routed::DeployStatus),
        "deploy.retry" => Ok(Routed::DeployRetry),
        "platform.plan" | "platform.apply" | "platform.status" => Ok(Routed::Platform {
            operation: request.operation.clone(),
            args: a.clone(),
        }),
        "git.tokens" => Ok(Routed::GitTokens),
        "grant.list" | "grant.request" | "grant.rotate" | "grant.revoke" => grant(request),
        "approval.wait" => Err("`approval.wait` shows a card, so it is called over MCP".into()),
        "mandate.propose" | "mandate.status" | "mandate.end" => Ok(Routed::Mandate {
            operation: request.operation.clone(),
            args: a.clone(),
        }),
        "events.wait" => Ok(Routed::Events { args: a.clone() }),
        "digest.show" => Ok(Routed::Digest),
        "apple.check" => Ok(Routed::AppleCheck),
        "ios.release" => Ok(Routed::IosRelease),
        "ios.status" => Ok(Routed::IosStatus),
        "session.start" => Ok(Routed::SessionStart { args: a.clone() }),
        "session.end" => Ok(Routed::SessionEnd { args: a.clone() }),
        "session.status" => Ok(Routed::SessionStatus { args: a.clone() }),
        "task.list" | "task.wait" | "task.claim" | "task.done" => Ok(Routed::Task {
            operation: request.operation.clone(),
            args: a.clone(),
        }),
        "change.list" => Ok(Routed::List {
            state: text(a, "state"),
        }),
        "change.evictable" => Ok(Routed::Evictable),
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
    /// For a deploy: its credentials, as environment; never logged or shown. For the merge job:
    /// [`COMMIT_AUTHOR`].
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

/// The Worker var, and the merge job's `env` key, naming the squash's author as `Name <email>`;
/// unset, merges are authored by `pigeon`. See
/// .claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md, the 2026-11-10
/// amendment.
pub const COMMIT_AUTHOR: &str = "COMMIT_AUTHOR";

/// What a merge commit's `Change-Approved-By:` says of approver `by`: the approval's kind, never
/// an identity, since `main` is published; D1's audit keeps `by`.
#[must_use]
pub fn public_approver(by: &str) -> String {
    match by.split_once(':') {
        Some(("policy" | "mandate", _)) => by.to_string(),
        Some(("grant", _)) => "grant:owner".into(),
        _ => "owner".into(),
    }
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
    /// For the bound job: the head's full set of code checks, for the merge gate; none for a
    /// head with no Rust input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub full: Vec<Check>,
    /// For the bound job: how much of `full` the merge gate runs; none from a job before the
    /// 2026-11-12 amendment on the merge gate, which gates in full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<crate::core::deploy::gate::Gate>,
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
    /// For the restack job: what it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restack: Option<changes::Restacked>,
    /// For a platform job: the plan or apply, its unit and its changes; see
    /// .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 4's amendment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<serde_json::Value>,
    /// For the eviction job: each candidate's verdict, `{number, granted, results}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evictions: Vec<serde_json::Value>,
    /// For the bound job: each overlapping open change the head would conflict with,
    /// `{number, paths}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forecast: Vec<serde_json::Value>,
    /// For the bound job of a change built on another: the commit of that change it is built on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_on: Option<String>,
    /// For the deploy job: `main`'s grants ledger, resolved; see
    /// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger: Option<crate::core::minting::Resolved>,
}

impl Report {
    /// A report of job `id` with only its outcome and the tail of `log`.
    #[must_use]
    pub fn of(id: &str, outcome: Outcome, log: &str) -> Self {
        Self {
            id: id.to_string(),
            outcome,
            log: tail(log).to_string(),
            violations: Vec::new(),
            checks: Vec::new(),
            full: Vec::new(),
            gate: None,
            merged: None,
            published: Vec::new(),
            workers: Vec::new(),
            build: None,
            images: None,
            tiers: None,
            restack: None,
            platform: None,
            evictions: Vec::new(),
            forecast: Vec::new(),
            built_on: None,
            ledger: None,
        }
    }
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
    /// `pigeon-sentinel`'s SHA-256, built beside it at `builds/<sha>/pigeon-sentinel`; see
    /// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentinel_sha256: Option<String>,
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
    if job == changes::RESTACK
        && report.outcome == Outcome::Passed
        && let Some(restack) = &report.restack
    {
        return Some(Event::Restacked {
            sha: sha.to_string(),
            restack: restack.clone(),
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
            full: report.full.clone(),
            tiers: report.tiers.clone(),
            gate: report.gate.clone(),
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

/// D1's plan approver for a change closed before its plan was authorized.
pub const CLOSED_UNAUTHORIZED: &str = "(closed unauthorized)";

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
    let approval = change.approval.as_ref().filter(|_| {
        matches!(
            change.state,
            changes::State::Approved | changes::State::Merged
        )
    });
    out.push(Statement {
        sql:
            "INSERT INTO changes (instance, repo, number, slug, title, plan, scope, proposer, state,
                head, plan_approver, approver, approved_sha, violations, created, updated, session,
                affects)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16,
                ?17)
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
            match (&change.plan_approval, change.state) {
                (Some(a), _) => a.by.clone().into(),
                // D1 refuses a plan approver only on `proposed`: a closed, never-authorized change
                // needs one; see .claude/decisions/2026-11-08-development-concurrent-agent-sessions.md §4.
                (None, changes::State::Closed) => CLOSED_UNAUTHORIZED.into(),
                (None, _) => Value::Null,
            },
            // D1 holds an approver only on an approved or merged head; a close keeps it in the object.
            opt(approval.map(|a| a.by.as_str())),
            opt(approval.and_then(|a| a.sha.as_deref())),
            Value::String(serde_json::to_string(&change.violations).unwrap_or_default()),
            now.into(),
            opt(change.session.as_deref()),
            Value::String(serde_json::to_string(&change.affects).unwrap_or_default()),
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

/// Whether `event`, just applied to `change`, ended its merge, so the queue's turn is released:
/// `Some(commit)` when merged, `Some(None)` when it failed or its Container was lost on every
/// attempt; a loss that is retried keeps the turn.
#[must_use]
pub fn merge_ended(event: &Event, change: &Change) -> Option<Option<String>> {
    match event {
        Event::Merged { commit, .. } => Some(commit.clone()),
        Event::MergeFailed { .. } => Some(None),
        Event::Lost { job, .. }
            if job == MERGE
                && change
                    .jobs
                    .iter()
                    .any(|j| j.name == MERGE && j.status == changes::JobStatus::Failed) =>
        {
            Some(None)
        }
        _ => None,
    }
}

/// Whether the events an alarm `applied` to `change`, a job's start lost among them, ended its
/// merge: as [`merge_ended`], for the last that did.
#[must_use]
pub fn merge_ended_by(applied: &[Event], change: &Change) -> Option<Option<String>> {
    applied.iter().rev().find_map(|e| merge_ended(e, change))
}

/// Whether a merge that ended, as [`merge_ended_by`] answers, gave its turn back without merging.
#[must_use]
pub fn released_without_merge(ended: Option<&Option<String>>) -> bool {
    matches!(ended, Some(None))
}

/// Whether `change` holds a merge turn it no longer waits on: no merge asked for meanwhile,
/// and none running or done.
#[must_use]
pub fn merge_dropped(change: &Change) -> bool {
    change.merge_after.is_none()
        && change.state != changes::State::Merged
        && !change.jobs.iter().any(|j| {
            j.name == MERGE
                && matches!(
                    j.status,
                    changes::JobStatus::Queued | changes::JobStatus::Running
                )
        })
}

/// Applies to `change`, a copy loaded before `journal` grew past `seen`, the events applied to the
/// stored change meanwhile; their effects are already pending, so they are dropped here. See the
/// 2026-11-11 amendment to `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.
pub fn catch_up(change: &mut Change, journal: &[changes::Event], seen: usize) {
    for e in journal.iter().skip(seen) {
        if let Ok((next, _)) = changes::step(change, e.clone()) {
            *change = next;
        }
    }
}

/// Whether `after`, saved over `before`, gives its place in the merge queue back: its merge
/// asked for was dropped, or it was just closed, waiting or holding the turn.
#[must_use]
pub fn leaves_merge_queue(before: Option<&Change>, after: &Change) -> bool {
    let dropped = before.is_some_and(|b| b.merge_after.is_some()) && merge_dropped(after);
    let closed = after.state == changes::State::Closed
        && before.is_some_and(|b| b.state != changes::State::Closed);
    dropped || closed
}

/// Why `change` gave its merge turn back without merging, for the `merge.released` event.
#[must_use]
pub fn release_reason(change: &Change) -> String {
    if let Some(v) = change.violations.first() {
        return v.clone();
    }
    if change.conflict.is_some() {
        return "it conflicts with main".into();
    }
    format!(
        "it is {} and carries no approval, so its merge can't start",
        change.state.name()
    )
}

/// Every open change's facts for eviction: its number, slug, state, session, head and when its
/// record last changed.
#[must_use]
pub fn evictable(instance: &str, repo: &str) -> Statement {
    Statement {
        sql: "SELECT number, slug, state, session, head, updated FROM changes
              WHERE instance = ?1 AND repo = ?2 AND state NOT IN ('merged', 'closed')
              ORDER BY number LIMIT 100",
        params: vec![instance.into(), repo.into()],
    }
}

/// Every open change: neither merged nor closed.
#[must_use]
pub fn open(instance: &str, repo: &str) -> Statement {
    Statement {
        sql: "SELECT number, state, session, scope, affects, plan, slug FROM changes
              WHERE instance = ?1 AND repo = ?2 AND state NOT IN ('merged', 'closed')
              ORDER BY number LIMIT 100",
        params: vec![instance.into(), repo.into()],
    }
}

/// The open changes in `rows`, as [`open`] answers them.
#[must_use]
pub fn open_rows(rows: &[serde_json::Value]) -> Vec<crate::sessions::Open> {
    let globs = |v: &serde_json::Value| -> Vec<String> {
        v.as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    };
    rows.iter()
        .filter_map(|r| {
            Some(crate::sessions::Open {
                number: u32::try_from(r["number"].as_u64()?).ok()?,
                state: r["state"].as_str()?.to_string(),
                session: r["session"].as_str().map(str::to_string),
                scope: globs(&r["scope"]),
                affects: globs(&r["affects"]),
                plan: r["plan"].as_str().unwrap_or_default().to_string(),
                slug: r["slug"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Every change, newest first, optionally in one state.
#[must_use]
pub fn list(instance: &str, repo: &str, state: Option<&str>) -> Statement {
    Statement {
        sql: "SELECT number, slug, title, state, head, session, updated FROM changes
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

/// The job that plans or applies a platform unit; see
/// .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 4's amendment.
pub const PLATFORM: &str = "platform";

/// The Worker secret holding the platform state's passphrase.
pub const PLATFORM_STATE_KEY_VAR: &str = "PLATFORM_STATE_KEY";

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

/// The bound job's variable holding the open changes whose scope meets the head's, as a JSON
/// array of `{number, branch, state}`; see the 2026-11-12 ADR §5's conflict forecasts.
pub const OVERLAPS: &str = "OVERLAPS";

/// The bound job's env: the branch of the open change the head is built on; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.
pub const AFTER: &str = "AFTER";

/// The restack job's env: the last head of the ended change the head was built on.
pub const AFTER_HEAD: &str = "AFTER_HEAD";

/// The job that judges eviction candidates with `main`'s eviction policies; see
/// `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4-§5.
pub const EVICTION: &str = "eviction";

/// The variable holding an eviction job's candidates, as a JSON array of `{number, fields}`.
pub const CANDIDATES: &str = "CANDIDATES";

/// The eviction job's ID for `main` at `sha` in the hour `hour` (Unix hours): one a run.
#[must_use]
pub fn eviction_id(sha: &str, hour: u64) -> String {
    format!("main-{}-{EVICTION}-{hour}", &sha[..12.min(sha.len())])
}

/// The spec for judging `candidates` against `main`'s policies at `sha`.
#[must_use]
pub fn eviction_spec(
    remote: &str,
    token: &str,
    sha: &str,
    candidates: &serde_json::Value,
    hour: u64,
) -> JobSpec {
    JobSpec {
        id: eviction_id(sha, hour),
        job: EVICTION.into(),
        env: std::collections::BTreeMap::from([(CANDIDATES.to_string(), candidates.to_string())]),
        ..deploy_spec(remote, token, sha, None, std::collections::BTreeMap::new())
    }
}

/// The eviction runs `MainRun` keeps: the one running, and the last verdicts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evictions {
    /// The job running, and when it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<(String, u64)>,
    /// When the last run ended, the head it judged, and each candidate's verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Judged>,
}

/// One eviction run's verdicts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judged {
    /// When it ended, in Unix seconds.
    pub at: u64,
    /// The job.
    pub id: String,
    /// Whether it judged; a failed run grants nothing.
    pub passed: bool,
    /// Each candidate's verdict.
    #[serde(default)]
    pub verdicts: Vec<serde_json::Value>,
}

impl Judged {
    /// The candidates a policy would evict.
    #[must_use]
    pub fn granted(&self) -> Vec<u64> {
        self.verdicts
            .iter()
            .filter(|v| v["granted"].is_string())
            .filter_map(|v| v["number"].as_u64())
            .collect()
    }
}

/// The job that mirrors `main` and the tags to GitHub, or a change's bounded head to its
/// branch there; see `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md` and
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §2.
pub const MIRROR: &str = changes::MIRROR;

/// The job that deletes a merged or closed change's branch from GitHub.
pub const UNMIRROR: &str = "unmirror";

/// The owner's break-glass: a change head's code checks on Cloudflare's `Checker`, which
/// `ci_rerun … on: cloudflare` asks for; see
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §5.
pub const BREAK_GLASS: &str = "ci.cloudflare";

/// `request` as the operation it asks for: `ci.rerun` with `on: cloudflare` is [`BREAK_GLASS`]
/// on its change, so it needs the owner like `change.close`.
#[must_use]
pub fn retarget(request: &Request) -> Request {
    let on = request.args.get("on").and_then(Value::as_str);
    if request.operation != "ci.rerun" || on != Some("cloudflare") {
        return request.clone();
    }
    let args = request
        .args
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "number" | "ticket"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Request {
        operation: BREAK_GLASS.into(),
        args,
    }
}

/// Whether `change` has a code check on its head that hasn't passed, which [`BREAK_GLASS`] would
/// run on Cloudflare.
#[must_use]
pub fn code_unpassed(change: &Change) -> bool {
    change.head.is_some()
        && change
            .jobs
            .iter()
            .any(|j| crate::actions::on_github(&j.argv) && j.status != changes::JobStatus::Passed)
}

/// Whether opening the CI window moves `change`: open, with a code check on its head queued or
/// running, as on GitHub; one that only failed stays for `ci_rerun`.
#[must_use]
pub fn window_moves(change: &Change) -> bool {
    use changes::JobStatus;
    change.head.is_some()
        && !matches!(
            change.state,
            changes::State::Merged | changes::State::Closed
        )
        && change.jobs.iter().any(|j| {
            crate::actions::on_github(&j.argv)
                && matches!(j.status, JobStatus::Queued | JobStatus::Running)
        })
}

/// [`BREAK_GLASS`] on `change`: `github` records its head as run on Cloudflare and drops its
/// dispatches; the answer is the events to apply, then the checks to start on the `Checker`.
///
/// # Errors
///
/// When the change has no open head with a code check that hasn't passed.
pub fn break_glass(
    change: &Change,
    github: &mut crate::actions::Actions,
) -> Result<(Vec<Event>, Vec<String>), String> {
    use changes::JobStatus;
    let Some(sha) = change.head.clone() else {
        return Err(format!("{} has no head", change.branch()));
    };
    if matches!(
        change.state,
        changes::State::Merged | changes::State::Closed
    ) {
        return Err(format!("{} is {}", change.branch(), change.state.name()));
    }
    if !code_unpassed(change) {
        return Err(format!(
            "{}'s code checks have all passed on {sha}",
            change.branch()
        ));
    }
    github.cloudflare = Some(sha.clone());
    github.dispatches.clear();
    let code = change
        .jobs
        .iter()
        .filter(|j| crate::actions::on_github(&j.argv));
    let mut events = Vec::new();
    let mut start = Vec::new();
    for j in code {
        match j.status {
            JobStatus::Running => events.push(Event::Lost {
                job: j.name.clone(),
                sha: sha.clone(),
            }),
            JobStatus::Queued => start.push(j.name.clone()),
            JobStatus::Failed => events.push(Event::Rerun {
                job: j.name.clone(),
            }),
            JobStatus::Passed | JobStatus::Cancelled => {}
        }
    }
    Ok((events, start))
}

/// Whether `job` on `change` runs on GitHub Actions: a code check, on a change whose head a
/// `mirror` job pushes, on a Worker that mirrors, while the owner's CI window is closed; see
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §1 and its
/// 2026-11-11 amendment.
#[must_use]
pub fn to_github(change: &Change, job: &str, mirrors: bool, window_open: bool) -> bool {
    mirrors
        && !window_open
        && change.jobs.iter().any(|j| j.name == MIRROR)
        && change
            .jobs
            .iter()
            .any(|j| j.name == job && crate::actions::on_github(&j.argv))
}

/// Where `change`'s `mirror` job is, for its dispatches.
#[must_use]
pub fn mirror_state(change: &Change) -> crate::actions::Mirror {
    use crate::actions::Mirror;
    match change
        .jobs
        .iter()
        .find(|j| j.name == MIRROR)
        .map(|j| j.status)
    {
        Some(changes::JobStatus::Passed) => Mirror::Passed,
        Some(changes::JobStatus::Queued | changes::JobStatus::Running) => Mirror::Open,
        _ => Mirror::Failed,
    }
}

/// `change`'s checks as `bound` recorded them, for a dispatch's `checks` input.
#[must_use]
pub fn checks_of(change: &Change) -> Vec<Check> {
    change
        .jobs
        .iter()
        .map(|j| Check {
            name: j.name.clone(),
            argv: j.argv.clone(),
        })
        .collect()
}

/// `event` with the [`MIRROR`] job among a passing bound job's checks, for a Worker that
/// mirrors; a refused head gains nothing, so it never leaves Cloudflare.
#[must_use]
pub fn with_mirror(event: Event) -> Event {
    match event {
        Event::Bounded {
            sha,
            violations,
            mut checks,
            full,
            tiers,
            gate,
        } => {
            if violations.is_empty() && !checks.iter().any(|c| c.name == MIRROR) {
                checks.push(Check {
                    name: MIRROR.into(),
                    argv: Vec::new(),
                });
            }
            Event::Bounded {
                sha,
                violations,
                checks,
                full,
                tiers,
                gate,
            }
        }
        other => other,
    }
}

/// Whether `after`, just merged or closed from `before`, has a branch on GitHub to delete: a
/// head the [`MIRROR`] job was started on.
#[must_use]
pub fn unmirror_due(before: Option<&Change>, after: &Change) -> bool {
    let done = |c: &Change| matches!(c.state, changes::State::Merged | changes::State::Closed);
    !before.is_some_and(done)
        && done(after)
        && after
            .jobs
            .iter()
            .any(|j| j.name == MIRROR && j.attempts > 0)
}

/// The spec that deletes `change`'s branch from `to`, with `to_token`.
#[must_use]
pub fn unmirror_spec(change: &Change, to: &str, to_token: &str) -> JobSpec {
    let sha = change.head.clone().unwrap_or_default();
    JobSpec {
        id: job_id(change.number, &sha, UNMIRROR),
        branch: change.branch(),
        number: change.number,
        job: UNMIRROR.into(),
        env: std::collections::BTreeMap::from([
            (MIRROR_TO.to_string(), to.to_string()),
            (MIRROR_TOKEN.to_string(), to_token.to_string()),
        ]),
        ..deploy_spec("", "", &sha, None, std::collections::BTreeMap::new())
    }
}

/// The mirror job's environment: GitHub's remote.
pub const MIRROR_TO: &str = "MIRROR_TO";

/// The mirror job's environment: the installation token for GitHub's remote.
pub const MIRROR_TOKEN: &str = "MIRROR_TOKEN";

/// A mirror job's ID: `main-<sha, 12>-mirror`.
#[must_use]
pub fn mirror_id(sha: &str) -> String {
    format!("main-{}-{MIRROR}", &sha[..12.min(sha.len())])
}

/// The spec for mirroring `sha` of `main` from `remote` (read with `token`) to `to`, pushed
/// with `to_token`.
#[must_use]
pub fn mirror_spec(remote: &str, token: &str, sha: &str, to: &str, to_token: &str) -> JobSpec {
    JobSpec {
        id: mirror_id(sha),
        job: MIRROR.into(),
        since: None,
        env: std::collections::BTreeMap::from([
            (MIRROR_TO.to_string(), to.to_string()),
            (MIRROR_TOKEN.to_string(), to_token.to_string()),
        ]),
        ..deploy_spec(remote, token, sha, None, std::collections::BTreeMap::new())
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

/// Why a failed job failed: its first violation, else its log's last non-empty line; see
/// `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`'s 2026-11-13
/// amendment.
#[must_use]
pub fn failure(report: &Report) -> String {
    report
        .violations
        .first()
        .cloned()
        .or_else(|| {
            report
                .log
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "the deploy failed with no output".into())
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
    /// The last pushes to `main`, oldest first, which order its commits without git; see
    /// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pushes: Vec<String>,
}

/// How many deploys `deploy.status` keeps.
pub const HISTORY: usize = 20;

/// How many pushes to `main` the pipeline keeps in order.
pub const PUSHES: usize = 100;

impl Pipeline {
    /// `main` moved to `sha`.
    pub fn pushed(&mut self, sha: &str) {
        if changes::is_sha(sha) {
            self.target = Some(sha.to_string());
            if self.pushes.last().map(String::as_str) != Some(sha) {
                self.pushes.push(sha.to_string());
                let over = self.pushes.len().saturating_sub(PUSHES);
                self.pushes.drain(..over);
            }
        }
    }

    /// Where the deploy carrying `merged`, a commit of `main`, stands; `Unknown` when it is
    /// older than the pushes kept.
    #[must_use]
    pub fn carrying(&self, merged: &str) -> crate::core::change::stage::Deploy {
        use crate::core::change::stage::Deploy;
        let order = |sha: &str| self.pushes.iter().position(|p| p == sha);
        let Some(at) = order(merged) else {
            return Deploy::Unknown;
        };
        let carries = |sha: &str| order(sha).is_some_and(|i| i >= at);
        if self.deployed.as_deref().is_some_and(carries) {
            return Deploy::Passed;
        }
        if self.running.as_ref().is_some_and(|(sha, _)| carries(sha)) {
            return Deploy::Running;
        }
        match self
            .history
            .iter()
            .find(|d| carries(&d.sha) && d.outcome == Outcome::Failed)
        {
            Some(d) => Deploy::Failed(
                d.violations
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "the deploy failed".into()),
            ),
            None => Deploy::Pending,
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
            pushes: Vec::new(),
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
            violations: if report.outcome == Outcome::Failed {
                vec![failure(report)]
            } else {
                report.violations.clone()
            },
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
    if job == BOUND
        || job == MERGE
        || job == changes::RESTACK
        || job == PLATFORM
        || job == MIRROR
        || job == UNMIRROR
        || job == EVICTION
    {
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
