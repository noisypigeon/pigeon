//! The merge gate and the merge's start: the head's full set of code checks before it is
//! squashed; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §5.

use super::lifecycle::Step;
use super::types::{Change, Check, Effect, GATE, Job, JobStatus, MERGE, State, conflicted, refuse};
use crate::core::deploy::gate::Gate;

impl Change {
    /// The head's full checks not yet passed on it, under their own name or as a gate job.
    fn gate_missing(&self) -> Vec<Check> {
        let passed = |name: &str| {
            self.jobs
                .iter()
                .any(|j| j.name == name && j.status == JobStatus::Passed)
        };
        let scoped = self.gate_full.is_none() && self.gate == Some(Gate::Scoped);
        self.full
            .iter()
            .filter(|c| !(scoped && whole_suite(c)))
            .filter(|c| !passed(&c.name) && !passed(&format!("{GATE}{}", c.name)))
            .cloned()
            .collect()
    }

    /// Why the head's merge gate runs the whole test suite; none when it is scoped. See the
    /// 2026-11-12 amendment on the merge gate to
    /// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
    #[must_use]
    pub fn gate_why(&self) -> Option<String> {
        if let Some(why) = &self.gate_full {
            return Some(why.clone());
        }
        match &self.gate {
            Some(Gate::Scoped) => None,
            Some(Gate::Full(why)) => Some(why.clone()),
            None => Some(Gate::default_reason()),
        }
    }

    /// After a gate job settles: a failure fails the change, naming the checks, and drops its
    /// merge; the last pass starts the merge.
    pub(super) fn gate_settled(mut self) -> (Change, Vec<Effect>) {
        if self.state != State::Approved {
            return (self, Vec::new());
        }
        let gate: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.name.starts_with(GATE))
            .collect();
        let open = gate.iter().any(|j| j.open());
        let failed: Vec<String> = gate
            .iter()
            .filter(|j| j.status == JobStatus::Failed)
            .map(|j| j.name.trim_start_matches(GATE).to_string())
            .collect();
        if !failed.is_empty() {
            self.violations = vec![format!(
                "the merge gate failed on {}: {}; push a fix, or ci_rerun the check",
                self.head.as_deref().unwrap_or("its head"),
                failed.join(", ")
            )];
            self.merge_after = None;
            self.state = State::Failed;
            return self.cancel_open();
        }
        if open {
            return (self, Vec::new());
        }
        match self.merge_after.take() {
            Some(by) => self.start_merge_or_drop(&by),
            None => (self, Vec::new()),
        }
    }

    /// Starts the merge job on an approved head for `by`, after the merge gate: the head's full
    /// checks not yet passed run first, as `gate: <check>` jobs, and the merge waits for them.
    pub(super) fn start_merge(mut self, by: &str) -> Step {
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
        if self.job(MERGE).is_some_and(Job::open) {
            return refuse(format!("{} is already being merged", self.branch()));
        }
        let missing = self.gate_missing();
        if !missing.is_empty() {
            self.merge_after = Some(by.to_string());
            if self
                .jobs
                .iter()
                .any(|j| j.name.starts_with(GATE) && j.open())
            {
                return Ok((self, Vec::new()));
            }
            let mut effects = Vec::new();
            for c in missing {
                let name = format!("{GATE}{}", c.name);
                self.jobs.retain(|j| j.name != name);
                effects.push(Effect::Start(name.clone()));
                self.jobs.push(Job::queued(&name, c.argv));
            }
            return Ok((self, effects));
        }
        self.jobs.retain(|j| j.name != MERGE);
        self.jobs.push(Job::queued(MERGE, Vec::new()));
        self.violations.clear();
        Ok((self, vec![Effect::Start(MERGE.into())]))
    }

    /// A merge asked for meanwhile, started now, or dropped when it can't start; its refusal
    /// comes before any change, so the change is as it was.
    pub(super) fn start_merge_or_drop(self, by: &str) -> (Change, Vec<Effect>) {
        match self.clone().start_merge(by) {
            Ok(next) => next,
            Err(_) => (self, Vec::new()),
        }
    }
}

/// Whether `c` runs the whole workspace's tests, which a scoped gate leaves out.
fn whole_suite(c: &Check) -> bool {
    c.argv.get(1).is_some_and(|v| v == "test") && c.argv.iter().any(|a| a == "--workspace")
}
