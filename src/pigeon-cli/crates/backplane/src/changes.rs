//! A change's lifecycle, from its proposal to its merge: a pure state machine the `Change`
//! Durable Object keeps in its storage and mirrors to D1; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1. Shared by both
//! builds, so every rule is tested natively.

use serde::{Deserialize, Serialize};

use crate::runs::MAX_ATTEMPTS;

/// The job that bounds a push: sentinel's change policy and `change.scope`, on the pushed tree.
pub const BOUND: &str = "bound";

/// The job that merges an approved head onto `main`, with a write token.
pub const MERGE: &str = "merge";

/// Where a change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Its plan waits for the owner.
    Proposed,
    /// Its plan is approved; nothing is pushed yet.
    Planned,
    /// Its head is being bounded.
    Pushed,
    /// Its head is inside its bounds; its checks run.
    Bounded,
    /// Its head broke a policy.
    Refused,
    /// Every check passed on its head.
    Checked,
    /// A check failed on its head.
    Failed,
    /// The owner approved its head.
    Approved,
    /// It is on `main`.
    Merged,
    /// It was closed unmerged.
    Closed,
}

impl State {
    /// Its name, as D1 stores it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Planned => "planned",
            Self::Pushed => "pushed",
            Self::Bounded => "bounded",
            Self::Refused => "refused",
            Self::Checked => "checked",
            Self::Failed => "failed",
            Self::Approved => "approved",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }

    fn done(self) -> bool {
        matches!(self, Self::Merged | Self::Closed)
    }
}

/// An approval: who, when, and for a merge the head SHA it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    /// The approver's identity.
    pub by: String,
    /// The SHA it covers; none for a plan's approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// When, in Unix seconds.
    pub at: u64,
}

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// Waiting for a Container.
    Queued,
    /// In a Container.
    Running,
    /// It passed.
    Passed,
    /// It failed, or used its attempts.
    Failed,
    /// A newer push or a failure stopped it.
    Cancelled,
}

/// One job on one head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// Its name: [`BOUND`], or a name `change.scope` gave.
    pub name: String,
    /// The command, run in the workspace; empty for [`BOUND`], which the Container knows.
    #[serde(default)]
    pub argv: Vec<String>,
    /// Where it is.
    pub status: JobStatus,
    /// How many times it has been dispatched.
    #[serde(default)]
    pub attempts: u32,
}

impl Job {
    fn queued(name: &str, argv: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            argv,
            status: JobStatus::Queued,
            attempts: 0,
        }
    }

    fn open(&self) -> bool {
        matches!(self.status, JobStatus::Queued | JobStatus::Running)
    }
}

/// A check `change.scope` named, as the bound job reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Its name.
    pub name: String,
    /// Its command.
    pub argv: Vec<String>,
}

/// A change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    /// Its number, as in `change/<number>-<slug>`.
    pub number: u32,
    /// Its slug.
    pub slug: String,
    /// Its title, a Conventional Commit subject.
    pub title: String,
    /// Its plan, as approved.
    pub plan: String,
    /// The paths it may touch, one glob each.
    pub scope: Vec<String>,
    /// Who proposed it.
    pub proposer: String,
    /// Where it is.
    pub state: State,
    /// Its pushed head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Its plan's approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_approval: Option<Approval>,
    /// Its merge approval, for its head only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    /// The head's jobs, [`BOUND`] first.
    #[serde(default)]
    pub jobs: Vec<Job>,
    /// The bound job's violations, when it refused the head, or why a merge failed.
    #[serde(default)]
    pub violations: Vec<String>,
    /// The commit on `main` it was merged as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_as: Option<String>,
    /// The head's approval tiers, as the bound job classified it; shown, not yet enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<crate::approvals::Tiers>,
}

/// Something that happened to a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// The owner approved its plan.
    ApprovePlan {
        /// Who.
        by: String,
        /// When.
        at: u64,
    },
    /// Its branch was pushed.
    Pushed {
        /// The new head.
        sha: String,
    },
    /// A job's Container started it.
    Started {
        /// The job.
        job: String,
        /// The head it ran on.
        sha: String,
    },
    /// The bound job finished.
    Bounded {
        /// The head it ran on.
        sha: String,
        /// Every violation; none passes.
        violations: Vec<String>,
        /// The checks `change.scope` named.
        checks: Vec<Check>,
        /// The head's approval tiers, shown only for now.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tiers: Option<crate::approvals::Tiers>,
    },
    /// A check finished.
    Finished {
        /// The job.
        job: String,
        /// The head it ran on.
        sha: String,
        /// Whether it passed.
        ok: bool,
    },
    /// A job's Container was lost.
    Lost {
        /// The job.
        job: String,
        /// The head it ran on.
        sha: String,
    },
    /// Someone asked for a finished job again.
    Rerun {
        /// The job.
        job: String,
    },
    /// The owner approved its head.
    Approve {
        /// Who.
        by: String,
        /// The head they approved.
        sha: String,
        /// When.
        at: u64,
    },
    /// A person asked for the approved head to be merged.
    Merge {
        /// Who.
        by: String,
    },
    /// Its approved head reached `main`.
    Merged {
        /// That head.
        sha: String,
        /// The commit on `main` it became.
        #[serde(default)]
        commit: Option<String>,
    },
    /// The merge job couldn't merge it; it stays approved.
    MergeFailed {
        /// The head it tried.
        sha: String,
        /// Why.
        reason: String,
    },
    /// It was closed unmerged.
    Close,
}

/// What the Durable Object must do after an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Dispatch this job on the head.
    Start(String),
    /// Stop these jobs' Containers.
    Cancel(Vec<String>),
}

/// Why an event was refused; the change is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

fn refuse<T>(why: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal(why.into()))
}

/// Whether `sha` is a full, lowercase git SHA.
#[must_use]
pub fn is_sha(sha: &str) -> bool {
    sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `change/<number>-<slug>`'s number and slug, if `branch` is one.
#[must_use]
pub fn branch(branch: &str) -> Option<(u32, String)> {
    let rest = branch
        .strip_prefix("refs/heads/")
        .unwrap_or(branch)
        .strip_prefix("change/")?;
    let (n, slug) = rest.split_once('-')?;
    let ok = !n.starts_with('0')
        && !slug.is_empty()
        && slug.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    ok.then(|| n.parse().ok().map(|n| (n, slug.to_string())))
        .flatten()
}

impl Change {
    /// A proposal, waiting for its plan's approval.
    #[must_use]
    pub fn propose(
        number: u32,
        slug: &str,
        title: &str,
        plan: &str,
        scope: Vec<String>,
        proposer: &str,
    ) -> Self {
        Self {
            number,
            slug: slug.to_string(),
            title: title.to_string(),
            plan: plan.to_string(),
            scope,
            proposer: proposer.to_string(),
            state: State::Proposed,
            head: None,
            plan_approval: None,
            approval: None,
            jobs: Vec::new(),
            violations: Vec::new(),
            merged_as: None,
            tiers: None,
        }
    }

    /// Its branch.
    #[must_use]
    pub fn branch(&self) -> String {
        format!("change/{}-{}", self.number, self.slug)
    }

    /// Applies `event`; what the Durable Object must do next.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] saying why; the change is then unchanged.
    pub fn apply(&mut self, event: Event) -> Result<Vec<Effect>, Refusal> {
        if self.state.done() && event != Event::Close {
            return refuse(format!(
                "{} is {}; nothing more happens to it",
                self.branch(),
                self.state.name()
            ));
        }
        match event {
            Event::ApprovePlan { by, at } => self.approve_plan(by, at),
            Event::Pushed { sha } => {
                if !self.on_head(&sha) {
                    self.tiers = None;
                }
                self.pushed(sha)
            }
            Event::Started { job, sha } => self.started(&job, &sha),
            Event::Bounded {
                sha,
                violations,
                checks,
                tiers,
            } => {
                if self.on_head(&sha) {
                    self.tiers = tiers;
                }
                self.bounded(&sha, violations, checks)
            }
            Event::Finished { job, sha, ok } => self.finished(&job, &sha, ok),
            Event::Lost { job, sha } => self.lost(&job, &sha),
            Event::Rerun { job } => self.rerun(&job),
            Event::Approve { by, sha, at } => self.approve(by, sha, at),
            Event::Merge { by } => self.merge(&by),
            Event::Merged { sha, commit } => self.merged(&sha, commit),
            Event::MergeFailed { sha, reason } => {
                if self.on_head(&sha) && self.state == State::Approved {
                    if let Some(j) = self.job(MERGE) {
                        j.status = JobStatus::Failed;
                    }
                    self.violations = vec![reason];
                }
                Ok(Vec::new())
            }
            Event::Close => {
                if self.state == State::Merged {
                    return refuse(format!("{} is merged", self.branch()));
                }
                self.state = State::Closed;
                Ok(self.cancel_open())
            }
        }
    }

    fn approve_plan(&mut self, by: String, at: u64) -> Result<Vec<Effect>, Refusal> {
        if self.state != State::Proposed {
            return refuse(format!("{}'s plan is already approved", self.branch()));
        }
        if by == self.proposer {
            return refuse("a plan is approved by someone other than its proposer");
        }
        self.plan_approval = Some(Approval { by, sha: None, at });
        self.state = State::Planned;
        Ok(Vec::new())
    }

    fn pushed(&mut self, sha: String) -> Result<Vec<Effect>, Refusal> {
        if !is_sha(&sha) {
            return refuse(format!("`{sha}` is not a commit SHA"));
        }
        if self.state == State::Proposed {
            return refuse(format!(
                "{} is pushed before its plan is approved",
                self.branch()
            ));
        }
        if self.head.as_deref() == Some(sha.as_str()) {
            return Ok(Vec::new());
        }
        let mut effects = self.cancel_open();
        self.head = Some(sha);
        self.approval = None;
        self.violations.clear();
        self.jobs = vec![Job::queued(BOUND, Vec::new())];
        self.state = State::Pushed;
        effects.push(Effect::Start(BOUND.into()));
        Ok(effects)
    }

    fn on_head(&self, sha: &str) -> bool {
        self.head.as_deref() == Some(sha)
    }

    fn job(&mut self, name: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.name == name)
    }

    fn started(&mut self, name: &str, sha: &str) -> Result<Vec<Effect>, Refusal> {
        if !self.on_head(sha) {
            return Ok(Vec::new());
        }
        match self.job(name) {
            Some(j) if j.status == JobStatus::Queued => {
                j.status = JobStatus::Running;
                j.attempts += 1;
                Ok(Vec::new())
            }
            Some(_) => refuse(format!("job `{name}` is not queued")),
            None => refuse(format!("no job `{name}`")),
        }
    }

    fn bounded(
        &mut self,
        sha: &str,
        violations: Vec<String>,
        checks: Vec<Check>,
    ) -> Result<Vec<Effect>, Refusal> {
        if !self.on_head(sha) || self.state != State::Pushed {
            return Ok(Vec::new());
        }
        let Some(bound) = self.job(BOUND) else {
            return refuse("no bound job");
        };
        if violations.is_empty() {
            bound.status = JobStatus::Passed;
            self.state = State::Bounded;
            let mut effects = Vec::new();
            for c in checks {
                if c.name == BOUND || self.jobs.iter().any(|j| j.name == c.name) {
                    continue;
                }
                effects.push(Effect::Start(c.name.clone()));
                self.jobs.push(Job::queued(&c.name, c.argv));
            }
            if effects.is_empty() {
                self.state = State::Checked;
            }
            Ok(effects)
        } else {
            bound.status = JobStatus::Failed;
            self.violations = violations;
            self.state = State::Refused;
            Ok(Vec::new())
        }
    }

    fn finished(&mut self, name: &str, sha: &str, ok: bool) -> Result<Vec<Effect>, Refusal> {
        if !self.on_head(sha) || name == BOUND {
            return Ok(Vec::new());
        }
        let Some(j) = self.job(name) else {
            return refuse(format!("no job `{name}`"));
        };
        if !j.open() {
            return Ok(Vec::new());
        }
        j.status = if ok {
            JobStatus::Passed
        } else {
            JobStatus::Failed
        };
        Ok(self.settle())
    }

    /// After a check settles: a failure fails the change and stops the rest; the last pass
    /// checks it.
    fn settle(&mut self) -> Vec<Effect> {
        if self.state != State::Bounded {
            return Vec::new();
        }
        if self.jobs.iter().any(|j| j.status == JobStatus::Failed) {
            self.state = State::Failed;
            return self.cancel_open();
        }
        if self.jobs.iter().all(|j| j.status == JobStatus::Passed) {
            self.state = State::Checked;
        }
        Vec::new()
    }

    fn lost(&mut self, name: &str, sha: &str) -> Result<Vec<Effect>, Refusal> {
        if !self.on_head(sha) {
            return Ok(Vec::new());
        }
        let Some(j) = self.job(name) else {
            return refuse(format!("no job `{name}`"));
        };
        if !j.open() {
            return Ok(Vec::new());
        }
        if j.attempts < MAX_ATTEMPTS {
            j.status = JobStatus::Queued;
            return Ok(vec![Effect::Start(name.to_string())]);
        }
        j.status = JobStatus::Failed;
        if name == MERGE {
            self.violations = vec![format!(
                "the merge job's Container was lost {MAX_ATTEMPTS} times; merge again"
            )];
            return Ok(Vec::new());
        }
        if name == BOUND {
            self.violations = vec![format!(
                "the bound job's Container was lost {MAX_ATTEMPTS} times"
            )];
            self.state = State::Refused;
            return Ok(Vec::new());
        }
        Ok(self.settle())
    }

    fn rerun(&mut self, name: &str) -> Result<Vec<Effect>, Refusal> {
        if !matches!(self.state, State::Failed | State::Checked | State::Refused) {
            return refuse(format!(
                "{} is {}; only a finished head's jobs run again",
                self.branch(),
                self.state.name()
            ));
        }
        let Some(j) = self.job(name) else {
            return refuse(format!("no job `{name}`"));
        };
        let bound = name == BOUND;
        *j = Job::queued(name, std::mem::take(&mut j.argv));
        if bound {
            self.jobs.retain(|j| j.name == BOUND);
            self.violations.clear();
            self.state = State::Pushed;
        } else {
            for j in &mut self.jobs {
                if j.status == JobStatus::Cancelled {
                    j.status = JobStatus::Queued;
                    j.attempts = 0;
                }
            }
            self.state = State::Bounded;
        }
        Ok(self
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Queued)
            .map(|j| Effect::Start(j.name.clone()))
            .collect())
    }

    fn approve(&mut self, by: String, sha: String, at: u64) -> Result<Vec<Effect>, Refusal> {
        if self.state != State::Checked {
            return refuse(format!(
                "{} is {}; only a checked head is approved",
                self.branch(),
                self.state.name()
            ));
        }
        if !self.on_head(&sha) {
            return refuse(format!(
                "{sha} is not {}'s head; approve the head that was checked",
                self.branch()
            ));
        }
        if by == self.proposer {
            return refuse("a change is approved by someone other than its proposer");
        }
        self.approval = Some(Approval {
            by,
            sha: Some(sha),
            at,
        });
        self.state = State::Approved;
        Ok(Vec::new())
    }

    fn merge(&mut self, by: &str) -> Result<Vec<Effect>, Refusal> {
        if self.state != State::Approved {
            return refuse(format!(
                "{} is {}; only an approved head is merged",
                self.branch(),
                self.state.name()
            ));
        }
        if self.job(MERGE).is_some_and(|j| j.open()) {
            return refuse(format!("{} is already being merged", self.branch()));
        }
        if by.is_empty() {
            return refuse("a merge names who asked for it");
        }
        self.jobs.retain(|j| j.name != MERGE);
        self.jobs.push(Job::queued(MERGE, Vec::new()));
        self.violations.clear();
        Ok(vec![Effect::Start(MERGE.into())])
    }

    fn merged(&mut self, sha: &str, commit: Option<String>) -> Result<Vec<Effect>, Refusal> {
        let approved = self
            .approval
            .as_ref()
            .and_then(|a| a.sha.as_deref())
            .is_some_and(|a| a == sha);
        if self.state != State::Approved || !self.on_head(sha) || !approved {
            return refuse(format!("{sha} is not {}'s approved head", self.branch()));
        }
        if let Some(j) = self.job(MERGE) {
            j.status = JobStatus::Passed;
        }
        self.merged_as = commit;
        self.state = State::Merged;
        Ok(Vec::new())
    }

    fn cancel_open(&mut self) -> Vec<Effect> {
        let names: Vec<String> = self
            .jobs
            .iter_mut()
            .filter(|j| j.open())
            .map(|j| {
                j.status = JobStatus::Cancelled;
                j.name.clone()
            })
            .collect();
        if names.is_empty() {
            Vec::new()
        } else {
            vec![Effect::Cancel(names)]
        }
    }
}
