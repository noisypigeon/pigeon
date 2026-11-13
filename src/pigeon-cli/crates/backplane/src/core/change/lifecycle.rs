//! A change's transitions, from its proposal to its merge: `step` and its dispatch; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1 and
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

use super::types::{
    Approval, BOUND, Change, Effect, Event, Job, JobStatus, MERGE, RESTACK, Refusal, State, is_sha,
    refuse,
};

/// A transition's answer: the next change and what the shell must do, or why it was refused.
pub type Step = Result<(Change, Vec<Effect>), Refusal>;

/// Applies `event` to `change`: the next change, and what the Durable Object must do next.
///
/// # Errors
///
/// A [`Refusal`] saying why; `change` is then unchanged.
pub fn step(change: &Change, event: Event) -> Step {
    change.clone().on(event)
}

impl Change {
    fn on(mut self, event: Event) -> Step {
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
                if !self.on_head(&sha) && self.job(RESTACK).is_some_and(Job::open) {
                    self.held = Some(sha);
                    return Ok((self, Vec::new()));
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
                full,
                tiers,
                gate,
            } => {
                if self.on_head(&sha) {
                    self.tiers = tiers;
                    self.full = full;
                    self.gate = gate;
                }
                self.bounded(&sha, violations, checks)
            }
            Event::Finished { job, sha, ok } => self.finished(&job, &sha, ok),
            Event::Lost { job, sha } => self.lost(&job, &sha),
            Event::Rerun { job } => self.rerun(&job),
            Event::Approve { by, sha, at } => self.approve(by, sha, at),
            Event::Merge { by } => self.merge(&by),
            Event::MergeFull { by } => {
                self.full_asked = Some(by.clone());
                self.merge(&by)
            }
            Event::Merged { sha, commit } => self.merged(&sha, commit),
            Event::MergeFailed { sha, reason } => {
                if !self.on_head(&sha) || self.state != State::Approved {
                    return Ok((self, Vec::new()));
                }
                self = self.set_status(MERGE, JobStatus::Failed);
                self.violations = vec![reason];
                // Behind, it restacks and merges again under its approval.
                match self.approval.as_ref().map(|a| a.by.clone()) {
                    Some(by) if self.behind.is_some() => Ok(self.restack_for_merge(&by)),
                    _ => Ok((self, Vec::new())),
                }
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

    fn approve_plan(mut self, by: String, at: u64) -> Step {
        if self.state != State::Proposed {
            return refuse(format!("{}'s plan is already approved", self.branch()));
        }
        if by == self.proposer {
            return refuse("a plan is approved by someone other than its proposer");
        }
        self.plan_approval = Some(Approval { by, sha: None, at });
        self.state = State::Planned;
        Ok((self, Vec::new()))
    }

    pub(super) fn pushed(self, sha: String) -> Step {
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
            return Ok((self, Vec::new()));
        }
        let (mut c, mut effects) = self.cancel_open();
        c.head = Some(sha);
        c.approval = None;
        c.conflict = None;
        c.violations.clear();
        c.jobs = vec![Job::queued(BOUND, Vec::new())];
        c.kept.clear();
        c.state = State::Pushed;
        effects.push(Effect::Start(BOUND.into()));
        Ok((c, effects))
    }

    /// A new head: its tiers and `behind` go with the old one.
    pub(super) fn push(mut self, sha: String) -> Step {
        if !self.on_head(&sha) {
            self.tiers = None;
            self.behind = None;
            self.full.clear();
            self.gate = None;
            self.gate_full = None;
        }
        self.pushed(sha)
    }

    /// A head held while a restack ran: pushed, or dropped when it is refused, as when it is no
    /// commit SHA.
    pub(super) fn push_held(self, sha: String) -> (Change, Vec<Effect>) {
        match self.clone().push(sha) {
            Ok(next) => next,
            Err(_) => (self, Vec::new()),
        }
    }

    fn approve(mut self, by: String, sha: String, at: u64) -> Step {
        if let Some(c) = &self.conflict {
            return refuse(super::types::conflicted(&self.branch(), c));
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
        // Approved by hand while a carry was pending: the carry is moot, and a parked merge starts.
        self.carried = None;
        match self.merge_after.take() {
            Some(by) => self.start_merge(&by),
            None => Ok((self, Vec::new())),
        }
    }

    fn merge(mut self, by: &str) -> Step {
        if by.is_empty() {
            return refuse("a merge names who asked for it");
        }
        let restacking = self.state == State::Approved && self.job(RESTACK).is_some_and(Job::open);
        // A head carrying an approval waits for the carry, whatever its checks have reached; see
        // the 2026-11-11 amendment to the concurrent-sessions ADR.
        let carrying = self.carried.is_some()
            && !matches!(
                self.state,
                State::Approved | State::Failed | State::Refused | State::Merged | State::Closed
            );
        if restacking || carrying {
            self.merge_after = Some(by.to_string());
            return Ok((self, Vec::new()));
        }
        if self.state == State::Approved && self.behind.is_some() {
            return Ok(self.restack_for_merge(by));
        }
        self.start_merge(by)
    }

    fn merged(mut self, sha: &str, commit: Option<String>) -> Step {
        let approved = self
            .approval
            .as_ref()
            .and_then(|a| a.sha.as_deref())
            .is_some_and(|a| a == sha);
        if self.state != State::Approved || !self.on_head(sha) || !approved {
            return refuse(format!("{sha} is not {}'s approved head", self.branch()));
        }
        self = self.set_status(MERGE, JobStatus::Passed);
        self.merged_as = commit;
        let (mut c, effects) = self.cancel_open();
        c.state = State::Merged;
        Ok((c, effects))
    }

    /// It, with every open job cancelled, and the effect that stops them.
    pub(super) fn cancel_open(mut self) -> (Change, Vec<Effect>) {
        let names: Vec<String> = self
            .jobs
            .iter_mut()
            .filter(|j| j.open())
            .map(|j| {
                j.status = JobStatus::Cancelled;
                j.name.clone()
            })
            .collect();
        let effects = if names.is_empty() {
            Vec::new()
        } else {
            vec![Effect::Cancel(names)]
        };
        (self, effects)
    }
}
