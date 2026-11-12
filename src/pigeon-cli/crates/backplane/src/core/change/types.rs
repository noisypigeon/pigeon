//! A change's types: its state, jobs, events and effects, as the `Change` Durable Object stores
//! them and mirrors to D1; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1.

use serde::{Deserialize, Serialize};

/// The job that bounds a push: sentinel's change policy and `change.scope`, on the pushed tree.
pub const BOUND: &str = "bound";

/// The job that merges an approved head onto `main`, with a write token.
pub const MERGE: &str = "merge";

/// What a merge gate job's name starts with: `gate: <check>`, one of the head's full set; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §5.
pub const GATE: &str = "gate: ";

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

    pub(super) fn done(self) -> bool {
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
    pub(super) fn queued(name: &str, argv: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            argv,
            status: JobStatus::Queued,
            attempts: 0,
        }
    }

    pub(super) fn open(&self) -> bool {
        matches!(self.status, JobStatus::Queued | JobStatus::Running)
    }

    /// Whether its result decides the change: every job but [`MIRROR`].
    pub(super) fn gates(&self) -> bool {
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
    /// Who asked to merge while its restack was pending, or while its merge gate runs; its
    /// merge starts once the approval carries over and the gate passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_after: Option<String>,
    /// The head's full set of code checks, which the merge gate runs; none for a head with no
    /// Rust input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub full: Vec<Check>,
    /// How much of `full` the merge gate runs, as the bound job reported it; none gates in full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<crate::core::deploy::gate::Gate>,
    /// Why the merge gate runs in full whatever the bound job reports, as after a clean restack;
    /// cleared by the session's next push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_full: Option<String>,
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
    /// Code checks and gate jobs that passed on the head before a disjoint, same-patch restack,
    /// carried to the new head; see the factory ADR's 2026-11-12 amendment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<Job>,
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
        /// The head's full set of code checks, for the merge gate.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        full: Vec<Check>,
        /// The head's approval tiers, shown only for now.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tiers: Option<crate::approvals::Tiers>,
        /// How much of `full` the merge gate runs; none from a bound job before the 2026-11-12
        /// amendment on the merge gate, which gates in full.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gate: Option<crate::core::deploy::gate::Gate>,
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

pub(super) fn conflicted(branch: &str, c: &Conflicted) -> String {
    let by = c
        .reconcile
        .map(|n| format!("; change {n} reconciles it"))
        .unwrap_or_default();
    format!(
        "{branch} conflicts with main in {}{by}; rebase it onto main and push",
        c.paths.join(", ")
    )
}

pub(super) fn refuse<T>(why: impl Into<String>) -> Result<T, Refusal> {
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
            full: Vec::new(),
            gate: None,
            gate_full: None,
            conflict: None,
            parent: None,
            held: None,
            grant: None,
            kept: Vec::new(),
        }
    }

    /// Its branch.
    #[must_use]
    pub fn branch(&self) -> String {
        format!("change/{}-{}", self.number, self.slug)
    }

    /// Whether `sha` is its head.
    pub(super) fn on_head(&self, sha: &str) -> bool {
        self.head.as_deref() == Some(sha)
    }

    /// Its job `name`, if it has one.
    pub(super) fn job(&self, name: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.name == name)
    }

    /// Where its job `name` sits among its jobs.
    pub(super) fn index(&self, name: &str) -> Option<usize> {
        self.jobs.iter().position(|j| j.name == name)
    }

    /// It, with job `name`'s status set to `status` if it has that job.
    pub(super) fn set_status(mut self, name: &str, status: JobStatus) -> Self {
        if let Some(i) = self.index(name) {
            self.jobs[i].status = status;
        }
        self
    }
}
