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

/// The job that replays an approved or checked head onto `main` after `main` moved; see
/// `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.
pub const RESTACK: &str = "restack";

/// The job that pushes a bounded head to GitHub as the change's branch; it never gates the
/// change. See `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §2.
pub const MIRROR: &str = "mirror";

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

    /// Whether its result decides the change: every job but [`MIRROR`].
    fn gates(&self) -> bool {
        self.name != MIRROR
    }
}

/// A check `change.scope` named, as the bound job reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Its name.
    pub name: String,
    /// Its command.
    pub argv: Vec<String>,
}

/// How a head stood to `main` after `main` moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// `main` is still its base.
    Current,
    /// None of its files changed on `main`.
    Disjoint,
    /// Some did, and git merged them.
    Clean,
    /// Git couldn't merge them.
    Conflicting,
}

/// What the restack job did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restacked {
    /// How the head stood.
    pub relation: Relation,
    /// The head it pushed; none when current or conflicting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Whether the change's own diff stayed the same.
    #[serde(default)]
    pub same_patch: bool,
    /// The paths git couldn't merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// The commit of `main` it replayed onto.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub onto: String,
    /// For a conflict: each path's changes on both sides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ConflictContext>,
    /// For a conflict: the messages of `main`'s commits that touched the paths.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub messages: String,
}

/// One conflicting path's changes on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictContext {
    /// The path.
    pub path: String,
    /// The subjects of `main`'s commits that touched it.
    #[serde(default)]
    pub commits: Vec<String>,
    /// `main`'s diff of it since the fork.
    #[serde(default)]
    pub base_diff: String,
    /// The change's diff of it since the fork.
    #[serde(default)]
    pub change_diff: String,
}

/// A head that conflicts with `main`, and the reconciliation planned for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflicted {
    /// The commit of `main` it conflicts with.
    pub onto: String,
    /// The paths in conflict.
    pub paths: Vec<String>,
    /// The reconciliation to propose; taken once proposed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<crate::reconcile::Proposal>,
    /// The reconciliation change, once proposed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconcile: Option<u32>,
}

/// An approval to carry onto a restacked head, if its patch and merge tier hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carried {
    /// Who approved.
    pub by: String,
    /// When.
    pub at: u64,
    /// The head they approved.
    pub from: String,
    /// That head's merge tier.
    pub merge: crate::approvals::Tier,
    /// The restacked head it carries to, once the restack job reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// One proof covering a change's remaining stages; see
/// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeGrant {
    /// Who gave the proof.
    pub by: String,
    /// When it lapses, in Unix seconds.
    pub expires: u64,
    /// Whether the owner keeps the merge for themselves.
    #[serde(default)]
    pub review: bool,
}

/// A change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The head's approval tiers, as the bound job classified it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<crate::approvals::Tiers>,
    /// The mandate it is carried out under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandate: Option<u32>,
    /// The agent session that proposed it, as it named itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// For a design change: the globs its ADR will govern.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
    /// The commit `main` moved to past this change's base; cleared by its next push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<String>,
    /// An approval carried across a restack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried: Option<Carried>,
    /// Who asked to merge while its restack was pending; its merge starts once the approval
    /// carries over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_after: Option<String>,
    /// Its head conflicts with `main`; cleared by its next push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflicted>,
    /// For a reconciliation: the change it reconciles or reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// A head pushed while its restack ran, applied once the restack reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// The owner's one proof for its remaining stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<ChangeGrant>,
}

/// Something that happened to a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    /// `main` moved to `main` past its base, by another change's merge.
    Behind {
        /// The commit `main` now holds.
        main: String,
    },
    /// The restack job replayed head `sha` onto `main`.
    Restacked {
        /// The head it replayed.
        sha: String,
        /// What it did.
        restack: Restacked,
    },
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

fn conflicted(branch: &str, c: &Conflicted) -> String {
    let by = c
        .reconcile
        .map(|n| format!("; change {n} reconciles it"))
        .unwrap_or_default();
    format!(
        "{branch} conflicts with main in {}{by}; rebase it onto main and push",
        c.paths.join(", ")
    )
}

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
            mandate: None,
            session: None,
            affects: Vec::new(),
            behind: None,
            carried: None,
            merge_after: None,
            conflict: None,
            parent: None,
            held: None,
            grant: None,
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
                if !self.on_head(&sha) && self.job(RESTACK).is_some_and(|j| j.open()) {
                    self.held = Some(sha);
                    return Ok(Vec::new());
                }
                self.push(sha)
            }
            Event::Behind { main } => self.behind(main),
            Event::Restacked { sha, restack } => self.restacked(&sha, &restack),
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
        self.conflict = None;
        self.violations.clear();
        self.jobs = vec![Job::queued(BOUND, Vec::new())];
        self.state = State::Pushed;
        effects.push(Effect::Start(BOUND.into()));
        Ok(effects)
    }

    /// A new head: its tiers and `behind` go with the old one.
    fn push(&mut self, sha: String) -> Result<Vec<Effect>, Refusal> {
        if !self.on_head(&sha) {
            self.tiers = None;
            self.behind = None;
        }
        self.pushed(sha)
    }

    /// `main` moved: a checked or approved head is restacked, keeping its approval to carry.
    fn behind(&mut self, main: String) -> Result<Vec<Effect>, Refusal> {
        let Some(head) = self.head.clone() else {
            return refuse(format!("{} has no head to be behind", self.branch()));
        };
        self.behind = Some(main);
        if !matches!(self.state, State::Checked | State::Approved)
            || self.job(RESTACK).is_some_and(|j| j.open())
            || self.job(MERGE).is_some_and(|j| j.open())
        {
            return Ok(Vec::new());
        }
        self.carried = match (&self.approval, &self.tiers) {
            (Some(a), Some(t)) if self.state == State::Approved => Some(Carried {
                by: a.by.clone(),
                at: a.at,
                from: head,
                merge: t.merge.tier,
                to: None,
            }),
            _ => None,
        };
        self.jobs.retain(|j| j.name != RESTACK);
        self.jobs.push(Job::queued(RESTACK, Vec::new()));
        Ok(vec![Effect::Start(RESTACK.into())])
    }

    /// The restack job's report on head `sha`, then a head held while it ran, if another.
    fn restacked(&mut self, sha: &str, r: &Restacked) -> Result<Vec<Effect>, Refusal> {
        let held = self.held.take();
        let mut effects = self.restack_report(sha, r)?;
        if let Some(h) = held
            && !self.on_head(&h)
        {
            self.carried = None;
            self.merge_after = None;
            effects.extend(self.push(h)?);
        }
        Ok(effects)
    }

    /// The restack job's report on head `sha`.
    fn restack_report(&mut self, sha: &str, r: &Restacked) -> Result<Vec<Effect>, Refusal> {
        let pushed_first = r.to.is_some() && self.head == r.to;
        if !self.on_head(sha) && !pushed_first {
            return Ok(Vec::new());
        }
        if let Some(j) = self.job(RESTACK) {
            j.status = JobStatus::Passed;
        }
        let carries = r.relation == Relation::Disjoint && r.same_patch;
        match (&r.relation, r.to.clone()) {
            (Relation::Current, _) => {
                self.behind = None;
                self.carried = None;
                match self.merge_after.take() {
                    Some(_) => self.start_merge(),
                    None => Ok(Vec::new()),
                }
            }
            (Relation::Conflicting, _) | (_, None) => {
                self.carried = None;
                self.merge_after = None;
                if r.relation == Relation::Conflicting {
                    self.conflict = Some(Conflicted {
                        onto: r.onto.clone(),
                        paths: r.conflicts.clone(),
                        proposal: Some(crate::reconcile::conflict(self, r)),
                        reconcile: None,
                    });
                }
                self.violations = vec![format!(
                    "{} conflicts with main in {}; rebase it onto main and push",
                    self.branch(),
                    r.conflicts.join(", ")
                )];
                Ok(Vec::new())
            }
            (_, Some(to)) => {
                match &mut self.carried {
                    Some(c) if carries && c.from == sha => c.to = Some(to.clone()),
                    _ => self.carried = None,
                }
                if pushed_first {
                    return Ok(self.carry());
                }
                self.tiers = None;
                self.behind = None;
                self.pushed(to)
            }
        }
    }

    /// Approves a restacked head that reached `checked` with the approval carried to it, when
    /// its merge tier is no higher than the one approved, then starts a merge asked for
    /// meanwhile; a merge whose approval didn't carry is dropped.
    fn carry(&mut self) -> Vec<Effect> {
        if self.state != State::Checked {
            return Vec::new();
        }
        let (Some(c), Some(t)) = (&self.carried, &self.tiers) else {
            self.carried = None;
            self.merge_after = None;
            return Vec::new();
        };
        if c.to.is_none() {
            return Vec::new();
        }
        let carries = c.to == self.head && t.merge.tier <= c.merge;
        if carries {
            self.approval = Some(Approval {
                by: c.by.clone(),
                sha: self.head.clone(),
                at: c.at,
            });
            self.state = State::Approved;
        }
        self.carried = None;
        match self.merge_after.take() {
            Some(_) if carries => self.start_merge().unwrap_or_default(),
            _ => Vec::new(),
        }
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
            if !self.jobs.iter().any(|j| j.gates() && j.open()) {
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
        if name == RESTACK {
            if let Some(j) = self.job(RESTACK) {
                j.status = if ok {
                    JobStatus::Passed
                } else {
                    JobStatus::Failed
                };
            }
            self.carried = None;
            return match self.held.take() {
                Some(h) => self.push(h),
                None => Ok(Vec::new()),
            };
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
        let mut gating = self.jobs.iter().filter(|j| j.gates());
        if gating.clone().any(|j| j.status == JobStatus::Failed) {
            self.state = State::Failed;
            return self.cancel_open();
        }
        if gating.all(|j| j.status == JobStatus::Passed) {
            self.state = State::Checked;
            return self.carry();
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
        if let Some(c) = &self.conflict {
            return refuse(conflicted(&self.branch(), c));
        }
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
        if by.is_empty() {
            return refuse("a merge names who asked for it");
        }
        let restacking =
            self.state == State::Approved && self.job(RESTACK).is_some_and(|j| j.open());
        // On `checked` the carry is already decided, so only a head still checking waits for it.
        let carrying =
            self.carried.is_some() && matches!(self.state, State::Pushed | State::Bounded);
        if restacking || carrying {
            self.merge_after = Some(by.to_string());
            return Ok(Vec::new());
        }
        self.start_merge()
    }

    /// Starts the merge job on an approved head.
    fn start_merge(&mut self) -> Result<Vec<Effect>, Refusal> {
        if let Some(c) = &self.conflict {
            return refuse(conflicted(&self.branch(), c));
        }
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
        let effects = self.cancel_open();
        self.state = State::Merged;
        Ok(effects)
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
