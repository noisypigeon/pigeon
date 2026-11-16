//! A head's jobs: started, bounded, finished, lost and run again.

use std::collections::BTreeMap;

use super::lifecycle::Step;
use super::types::{
    BOUND, Change, Check, Effect, GATE, Job, JobStatus, MERGE, MIRROR, RESTACK, State, refuse,
};

use crate::core::platform::runs::MAX_ATTEMPTS;

/// Failed starts before a job fails; see
/// `.claude/decisions/2026-11-15-development-job-start-failures-backoff.md` §1.
pub const MAX_START_FAILURES: u32 = 8;

/// Seconds before the next start after `failures` failed starts: 15, 30, 60, 120, then 240.
#[must_use]
pub fn start_backoff(failures: u32) -> u64 {
    match failures {
        0 | 1 => 15,
        2 => 30,
        3 => 60,
        4 => 120,
        _ => 240,
    }
}

/// Seconds before a lost job's second and third attempts.
pub const LOST_BACKOFF: [u64; 2] = [30, 60];

/// Seconds a job may sit queued with no attempt before it is dispatched again.
pub const STALL_SECS: u64 = 5 * 60;

/// `change`'s queued jobs that no start in `pending` names and that run `elsewhere` does not
/// claim: each is dispatched again; see the job-start ADR's yield-end amendment.
#[must_use]
pub fn undispatched(
    change: &Change,
    pending: &[&str],
    elsewhere: impl Fn(&str) -> bool,
) -> Vec<String> {
    change
        .jobs
        .iter()
        .filter(|j| j.status == JobStatus::Queued && !pending.contains(&j.name.as_str()))
        .filter(|j| !elsewhere(&j.name))
        .map(|j| j.name.clone())
        .collect()
}

/// When each of `change`'s jobs queued with no attempt and no failed start was first seen,
/// `since` kept and added to at `now`; and those queued [`STALL_SECS`] or more, which restart.
#[must_use]
pub fn stalled(
    change: &Change,
    since: &BTreeMap<String, u64>,
    now: u64,
) -> (BTreeMap<String, u64>, Vec<String>) {
    let head = change.head.as_deref().unwrap_or_default();
    let mut kept = BTreeMap::new();
    let mut out = Vec::new();
    for j in &change.jobs {
        if j.status != JobStatus::Queued || j.attempts > 0 || j.start_failures > 0 {
            continue;
        }
        let key = format!("{head}/{}", j.name);
        let first = since.get(&key).copied().unwrap_or(now);
        if now.saturating_sub(first) >= STALL_SECS {
            out.push(j.name.clone());
            kept.insert(key, now);
        } else {
            kept.insert(key, first);
        }
    }
    (kept, out)
}

impl Change {
    pub(super) fn started(mut self, name: &str, sha: &str) -> Step {
        if !self.on_head(sha) {
            return Ok((self, Vec::new()));
        }
        match self.index(name) {
            Some(i) if self.jobs[i].status == JobStatus::Queued => {
                self.jobs[i].status = JobStatus::Running;
                self.jobs[i].attempts += 1;
                self.jobs[i].start_failures = 0;
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
        self.carried_checks.clear();
        let mut effects = Vec::new();
        for c in checks {
            if c.name == BOUND || self.jobs.iter().any(|j| j.name == c.name) {
                continue;
            }
            // Passed on the head before a disjoint, same-patch restack: not run again.
            if let Some(k) = kept.iter().find(|k| k.name == c.name && k.argv == c.argv) {
                self.jobs.push(k.clone());
                self.carried_checks.push(k.name.clone());
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
                self.carried_checks.push(k.name.clone());
                self.jobs.push(k);
            }
        }
        let (c, early) = self.early_gate();
        self = c;
        effects.extend(early);
        if self.jobs.iter().any(|j| j.gates() && j.open()) {
            return Ok((self, effects));
        }
        self.state = State::Checked;
        let (c, more) = self.checked();
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
            if !ok {
                // A parked merge can't start on a head that didn't restack; its turn goes back.
                self.merge_after = None;
            }
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
            // A failed head can't merge: its parked merge, and so its turn, goes back; see the
            // concurrent-sessions ADR's 2026-11-16 amendment.
            self.merge_after = None;
            self.state = State::Failed;
            return self.cancel_open();
        }
        if gating.all(|j| j.status == JobStatus::Passed) {
            self.state = State::Checked;
            return self.checked();
        }
        (self, Vec::new())
    }

    /// A head just checked: a carried approval applies, then a head left `behind` restacks.
    fn checked(self) -> (Change, Vec<Effect>) {
        let (c, mut effects) = self.carry();
        let (c, more) = c.restack_if_behind();
        effects.extend(more);
        (c, effects)
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
        self.jobs[i].last_error = Some("its Container was lost while it ran".into());
        let attempts = self.jobs[i].attempts;
        if attempts < MAX_ATTEMPTS {
            self.jobs[i].status = JobStatus::Queued;
            let secs = LOST_BACKOFF[usize::try_from(attempts.saturating_sub(1))
                .unwrap_or(0)
                .min(1)];
            let job = name.to_string();
            return Ok((self, vec![Effect::StartAfter { job, secs }]));
        }
        self.given_up(
            i,
            name,
            sha,
            &format!("Container was lost {MAX_ATTEMPTS} times"),
        )
    }

    /// A queued job whose Container didn't start: it starts again after a backoff, and fails
    /// after [`MAX_START_FAILURES`], as a lost one does.
    pub(super) fn start_failed(mut self, name: &str, sha: &str, why: &str) -> Step {
        if !self.on_head(sha) {
            return Ok((self, Vec::new()));
        }
        let Some(i) = self.index(name) else {
            return refuse(format!("no job `{name}`"));
        };
        if self.jobs[i].status != JobStatus::Queued {
            return Ok((self, Vec::new()));
        }
        let why: String = why.chars().take(300).collect();
        self.jobs[i].start_failures += 1;
        self.jobs[i].last_error = Some(why.clone());
        let failures = self.jobs[i].start_failures;
        if failures < MAX_START_FAILURES {
            let job = name.to_string();
            let secs = start_backoff(failures);
            return Ok((self, vec![Effect::StartAfter { job, secs }]));
        }
        let what = format!("Container failed to start {MAX_START_FAILURES} times: {why}");
        self.given_up(i, name, sha, &what)
    }

    /// Job `i`, `name`, ends for `what`: a restack as a failed one, a merge giving its turn
    /// back, a bound refusing the head, and a check failing.
    fn given_up(mut self, i: usize, name: &str, sha: &str, what: &str) -> Step {
        if name == RESTACK {
            let (mut c, effects) = self.finished(RESTACK, sha, false)?;
            c.violations.push(format!(
                "the restack job's {what}; rebase onto main and push, or it restacks after the next merge"
            ));
            return Ok((c, effects));
        }
        self.jobs[i].status = JobStatus::Failed;
        if name == MERGE {
            self.violations = vec![format!("the merge job's {what}; merge again")];
            return Ok((self, Vec::new()));
        }
        if name == BOUND {
            self.violations = vec![format!("the bound job's {what}")];
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
        // Queued and never started: dispatched again, on any head (job-start-failures ADR §4).
        if self
            .job(name)
            .is_some_and(|j| j.status == JobStatus::Queued && j.attempts == 0)
        {
            return Ok((self, vec![Effect::Start(name.to_string())]));
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
