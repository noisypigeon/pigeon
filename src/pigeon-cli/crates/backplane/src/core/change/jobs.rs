//! A head's jobs: started, bounded, finished, lost and run again.

use super::lifecycle::Step;
use super::types::{
    BOUND, Change, Check, Effect, GATE, Job, JobStatus, MERGE, MIRROR, RESTACK, State, refuse,
};
use crate::runs::MAX_ATTEMPTS;

impl Change {
    pub(super) fn started(mut self, name: &str, sha: &str) -> Step {
        if !self.on_head(sha) {
            return Ok((self, Vec::new()));
        }
        match self.index(name) {
            Some(i) if self.jobs[i].status == JobStatus::Queued => {
                self.jobs[i].status = JobStatus::Running;
                self.jobs[i].attempts += 1;
                Ok((self, Vec::new()))
            }
            Some(_) => refuse(format!("job `{name}` is not queued")),
            None => refuse(format!("no job `{name}`")),
        }
    }

    pub(super) fn bounded(
        mut self,
        sha: &str,
        violations: Vec<String>,
        checks: Vec<Check>,
    ) -> Step {
        if !self.on_head(sha) || self.state != State::Pushed {
            return Ok((self, Vec::new()));
        }
        let Some(bound) = self.index(BOUND) else {
            return refuse("no bound job");
        };
        if !violations.is_empty() {
            self.jobs[bound].status = JobStatus::Failed;
            self.violations = violations;
            self.state = State::Refused;
            return Ok((self, Vec::new()));
        }
        self.jobs[bound].status = JobStatus::Passed;
        self.state = State::Bounded;
        let kept = self.kept.split_off(0);
        let mut effects = Vec::new();
        for c in checks {
            if c.name == BOUND || self.jobs.iter().any(|j| j.name == c.name) {
                continue;
            }
            // Passed on the head before a disjoint, same-patch restack: not run again.
            if let Some(k) = kept.iter().find(|k| k.name == c.name && k.argv == c.argv) {
                self.jobs.push(k.clone());
                continue;
            }
            effects.push(Effect::Start(c.name.clone()));
            self.jobs.push(Job::queued(&c.name, c.argv));
        }
        // A kept gate job counts toward the merge gate while its check is still in the full set.
        for k in kept {
            let in_full = k
                .name
                .strip_prefix(GATE)
                .is_some_and(|n| self.full.iter().any(|f| f.name == n && f.argv == k.argv));
            if in_full && !self.jobs.iter().any(|j| j.name == k.name) {
                self.jobs.push(k);
            }
        }
        if self.jobs.iter().any(|j| j.gates() && j.open()) {
            return Ok((self, effects));
        }
        self.state = State::Checked;
        let (c, more) = self.carry();
        effects.extend(more);
        Ok((c, effects))
    }

    pub(super) fn finished(mut self, name: &str, sha: &str, ok: bool) -> Step {
        if !self.on_head(sha) || name == BOUND {
            return Ok((self, Vec::new()));
        }
        let status = if ok {
            JobStatus::Passed
        } else {
            JobStatus::Failed
        };
        if name == RESTACK {
            self = self.set_status(RESTACK, status);
            self.carried = None;
            return Ok(match self.held.take() {
                Some(h) => self.push_held(h),
                None => (self, Vec::new()),
            });
        }
        let Some(i) = self.index(name) else {
            return refuse(format!("no job `{name}`"));
        };
        if !self.jobs[i].open() {
            return Ok((self, Vec::new()));
        }
        self.jobs[i].status = status;
        if name.starts_with(GATE) {
            return Ok(self.gate_settled());
        }
        Ok(self.settle())
    }

    /// After a check settles: a failure fails the change and stops the rest; the last pass
    /// checks it.
    pub(super) fn settle(mut self) -> (Change, Vec<Effect>) {
        if self.state != State::Bounded {
            return (self, Vec::new());
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
        (self, Vec::new())
    }

    pub(super) fn lost(mut self, name: &str, sha: &str) -> Step {
        if !self.on_head(sha) {
            return Ok((self, Vec::new()));
        }
        let Some(i) = self.index(name) else {
            return refuse(format!("no job `{name}`"));
        };
        if !self.jobs[i].open() {
            return Ok((self, Vec::new()));
        }
        if self.jobs[i].attempts < MAX_ATTEMPTS {
            self.jobs[i].status = JobStatus::Queued;
            return Ok((self, vec![Effect::Start(name.to_string())]));
        }
        self.jobs[i].status = JobStatus::Failed;
        if name == MERGE {
            self.violations = vec![format!(
                "the merge job's Container was lost {MAX_ATTEMPTS} times; merge again"
            )];
            return Ok((self, Vec::new()));
        }
        if name == BOUND {
            self.violations = vec![format!(
                "the bound job's Container was lost {MAX_ATTEMPTS} times"
            )];
            self.state = State::Refused;
            return Ok((self, Vec::new()));
        }
        if name.starts_with(GATE) {
            return Ok(self.gate_settled());
        }
        Ok(self.settle())
    }

    pub(super) fn rerun(mut self, name: &str) -> Step {
        // The mirror job never gates, so it runs again on any head that passed `bound`.
        if name == MIRROR
            && let Some(i) = self.index(MIRROR)
        {
            if self.jobs[i].open() {
                return Ok((self, Vec::new()));
            }
            self.jobs[i] = Job::queued(MIRROR, Vec::new());
            return Ok((self, vec![Effect::Start(MIRROR.into())]));
        }
        if name == MERGE {
            return refuse(format!(
                "a merge runs again through the merge queue: change_merge {}",
                self.number
            ));
        }
        if !matches!(self.state, State::Failed | State::Checked | State::Refused) {
            return refuse(format!(
                "{} is {}; only a finished head's jobs run again",
                self.branch(),
                self.state.name()
            ));
        }
        let Some(i) = self.index(name) else {
            return refuse(format!("no job `{name}`"));
        };
        let argv = self.jobs[i].argv.clone();
        self.jobs[i] = Job::queued(name, argv);
        if name == BOUND {
            self.jobs.retain(|j| j.name == BOUND);
            self.violations.clear();
            self.state = State::Pushed;
        } else {
            self.jobs
                .iter_mut()
                .filter(|j| j.status == JobStatus::Cancelled)
                .for_each(|j| {
                    j.status = JobStatus::Queued;
                    j.attempts = 0;
                });
            self.state = State::Bounded;
        }
        let effects = self
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Queued)
            .map(|j| Effect::Start(j.name.clone()))
            .collect();
        Ok((self, effects))
    }
}
