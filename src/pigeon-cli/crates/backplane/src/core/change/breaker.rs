//! The breaker: what a change's heads show of a loop it can't fix, counted per head; see
//! `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §4. In
//! shadow it only says what would trip.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::types::{BOUND, Change, JobStatus, MERGE, MIRROR, RESTACK, State};
use crate::core::brief::Budget;
use crate::sessions::Class;

/// How many heads are kept.
pub const HEADS: usize = 10;

/// B1: the same check failing on this many heads in a row.
pub const SAME_CHECK: usize = 3;

/// B2: the same failure fingerprint on this many heads.
pub const SAME_FAILURE: usize = 2;

/// B3: the same policy refusing this many heads in a row.
pub const SAME_REFUSAL: usize = 3;

/// B4: this many heads pushed without one reaching `checked`.
pub const UNCHECKED: usize = 8;

/// B5: one job on one head run this many times.
pub const RUNS: u32 = 3;

/// B6: this long, in seconds, across [`STUCK_HEADS`] or more heads without reaching `checked`.
pub const STUCK_FOR: u64 = 3 * 60 * 60;

/// B6's heads.
pub const STUCK_HEADS: usize = 3;

/// The thresholds that differ by class: an opus session's, or a brief's; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// B1's heads.
    pub same_check: usize,
    /// B2's heads.
    pub same_failure: usize,
    /// B4's heads.
    pub unchecked: usize,
    /// The brief's budget, for an executor's change.
    pub budget: Option<Budget>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            same_check: SAME_CHECK,
            same_failure: SAME_FAILURE,
            unchecked: UNCHECKED,
            budget: None,
        }
    }
}

impl Limits {
    /// A brief's change's, carried out by `model` within `budget`.
    #[must_use]
    pub const fn brief(model: Class, budget: Budget) -> Self {
        let (same_check, unchecked) = match model {
            Class::Haiku => (2, 3),
            Class::Sonnet => (2, 4),
            Class::Opus => (SAME_CHECK, UNCHECKED),
        };
        Self {
            same_check,
            same_failure: SAME_FAILURE,
            unchecked,
            budget: Some(budget),
        }
    }
}

/// What one head showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    /// Its SHA.
    pub sha: String,
    /// When it was first seen, in Unix seconds.
    pub at: u64,
    /// The checks that failed on it.
    #[serde(default)]
    pub failed: BTreeSet<String>,
    /// Their failures' fingerprints, as `<check>: <first error line, normalized>`.
    #[serde(default)]
    pub prints: BTreeSet<String>,
    /// The policy that refused it, normalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// Each job's dispatches on it.
    #[serde(default)]
    pub runs: BTreeMap<String, u32>,
    /// Whether it reached `checked`.
    #[serde(default)]
    pub checked: bool,
}

/// One signal tripping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trip {
    /// `B1` to `B6`.
    pub signal: String,
    /// What it saw, in a line.
    pub why: String,
}

/// A change's heads, newest last, and the trips already reported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breaker {
    /// The last [`HEADS`] heads.
    #[serde(default)]
    pub heads: VecDeque<Head>,
    /// The trips reported, by `<signal> <why>`, so each is reported once.
    #[serde(default)]
    pub reported: BTreeSet<String>,
}

/// The first line of `tail` that reads as an error, else its last non-empty line, with numbers,
/// hashes and paths normalized so the same failure on two heads prints alike.
#[must_use]
pub fn fingerprint(tail: &str) -> Option<String> {
    let lines: Vec<&str> = tail
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let first = lines
        .iter()
        .find(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("error") || l.contains("panicked") || l.contains(" failed")
        })
        .or(lines.last())?;
    let words: Vec<String> = first
        .split_whitespace()
        .map(|w| {
            if w.contains('/') || w.contains('\\') {
                "<path>".into()
            } else if w.len() >= 7 && w.chars().all(|c| c.is_ascii_hexdigit()) {
                "<hash>".into()
            } else {
                w.chars()
                    .map(|c| if c.is_ascii_digit() { '#' } else { c })
                    .collect()
            }
        })
        .collect();
    let mut out = words.join(" ");
    out.truncate(160);
    Some(out)
}

fn decides(name: &str) -> bool {
    ![BOUND, MIRROR, RESTACK, MERGE].contains(&name)
}

/// Notes `after`, given the failed jobs' log tails in `tails`, at `now`: the next breaker.
#[must_use]
pub fn observe(b: &Breaker, after: &Change, tails: &BTreeMap<String, String>, now: u64) -> Breaker {
    let mut b = b.clone();
    let Some(sha) = after.head.clone() else {
        return b;
    };
    if b.heads.back().map(|h| h.sha.as_str()) != Some(sha.as_str()) {
        b.heads.push_back(Head {
            sha,
            at: now,
            ..Head::default()
        });
        while b.heads.len() > HEADS {
            b.heads.pop_front();
        }
    }
    let Some(head) = b.heads.back_mut() else {
        return b;
    };
    for j in after.jobs.iter().filter(|j| decides(&j.name)) {
        let runs = head.runs.entry(j.name.clone()).or_default();
        *runs = (*runs).max(j.attempts);
        if j.status == JobStatus::Failed {
            head.failed.insert(j.name.clone());
            if let Some(fp) = tails.get(&j.name).and_then(|t| fingerprint(t)) {
                head.prints.insert(format!("{}: {fp}", j.name));
            }
        }
    }
    if after.state == State::Refused {
        head.refused = after.violations.first().and_then(|v| fingerprint(v));
    }
    if matches!(
        after.state,
        State::Checked | State::Approved | State::Merged
    ) {
        head.checked = true;
    }
    b
}

/// The signals `b` trips at `now`, under `limits`.
#[must_use]
pub fn signals(b: &Breaker, now: u64, limits: &Limits) -> Vec<Trip> {
    let mut out = Vec::new();
    let trip = |signal: &str, why: String| Trip {
        signal: signal.into(),
        why,
    };
    let recent: Vec<&Head> = b.heads.iter().rev().collect();
    let same_check = limits.same_check;
    if recent.len() >= same_check {
        let last = &recent[..same_check];
        if let Some(job) = last[0]
            .failed
            .iter()
            .find(|j| last.iter().all(|h| h.failed.contains(*j)))
        {
            out.push(trip(
                "B1",
                format!("`{job}` failed on {same_check} heads in a row"),
            ));
        }
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for h in &b.heads {
        for p in &h.prints {
            *seen.entry(p.as_str()).or_default() += 1;
        }
    }
    if let Some((p, n)) = seen.iter().find(|(_, n)| **n >= limits.same_failure) {
        out.push(trip("B2", format!("the same failure on {n} heads: {p}")));
    }
    if recent.len() >= SAME_REFUSAL {
        let last = &recent[..SAME_REFUSAL];
        if let Some(rule) = last[0].refused.as_ref()
            && last.iter().all(|h| h.refused.as_ref() == Some(rule))
        {
            out.push(trip(
                "B3",
                format!("the same policy refused {SAME_REFUSAL} heads: {rule}"),
            ));
        }
    }
    let unchecked: Vec<&Head> = recent.iter().copied().take_while(|h| !h.checked).collect();
    if unchecked.len() >= limits.unchecked {
        out.push(trip(
            "B4",
            format!(
                "{} heads pushed without one reaching checked",
                unchecked.len()
            ),
        ));
    }
    if let Some((job, n)) = b
        .heads
        .back()
        .and_then(|h| h.runs.iter().find(|(_, n)| **n >= RUNS))
    {
        out.push(trip("B5", format!("`{job}` ran {n} times on one head")));
    }
    if unchecked.len() >= STUCK_HEADS
        && let Some(first) = unchecked.last()
        && now.saturating_sub(first.at) >= STUCK_FOR
    {
        out.push(trip(
            "B6",
            format!(
                "{} heads over {} hours without reaching checked",
                unchecked.len(),
                now.saturating_sub(first.at) / 3600
            ),
        ));
    }
    if let Some(budget) = limits.budget {
        out.extend(spent(b, now, budget));
    }
    out
}

/// The budget's trip: more heads than it allows, or its hours gone with the last head unchecked.
fn spent(b: &Breaker, now: u64, budget: Budget) -> Option<Trip> {
    let heads = b.heads.len();
    let why = if heads > budget.heads as usize {
        format!("{heads} heads, over the brief's {}", budget.heads)
    } else {
        let first = b.heads.front()?;
        let hours = now.saturating_sub(first.at) / 3600;
        if b.heads.back().is_some_and(|h| h.checked) || hours < u64::from(budget.hours) {
            return None;
        }
        format!("{hours} hours, over the brief's {}", budget.hours)
    };
    Some(Trip {
        signal: "budget".into(),
        why,
    })
}

/// The trips in `b` at `now`, under `limits`, not yet reported, and the breaker with them marked
/// reported.
#[must_use]
pub fn fresh(b: &Breaker, now: u64, limits: &Limits) -> (Breaker, Vec<Trip>) {
    let mut b = b.clone();
    let new: Vec<Trip> = signals(&b, now, limits)
        .into_iter()
        .filter(|t| b.reported.insert(format!("{} {}", t.signal, t.why)))
        .collect();
    (b, new)
}

#[cfg(test)]
mod tests {
    use super::super::types::Job;
    use super::*;

    fn change(sha: &str, state: State, jobs: &[(&str, JobStatus, u32)]) -> Change {
        let mut c = Change::propose(7, "x", "feat: x", "p", vec!["**".into()], "agent");
        c.head = Some(sha.into());
        c.state = state;
        c.jobs = jobs
            .iter()
            .map(|(n, s, a)| {
                let mut j = Job::queued(n, Vec::new());
                j.status = *s;
                j.attempts = *a;
                j
            })
            .collect();
        c
    }

    fn tails(job: &str, tail: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(job.to_string(), tail.to_string())])
    }

    fn names(t: &[Trip]) -> Vec<&str> {
        t.iter().map(|t| t.signal.as_str()).collect()
    }

    #[test]
    fn a_fingerprint_ignores_numbers_hashes_and_paths() {
        let a = fingerprint("  compiling\nerror[E0599]: no method `x` at src/a.rs:12:5\n");
        let b = fingerprint("error[E0599]: no method `x` at src/b.rs:40:9");
        assert_eq!(a, b);
        assert_eq!(a.as_deref(), Some("error[E####]: no method `x` at <path>"));
        assert_eq!(
            fingerprint("ok\n\n"),
            Some("ok".into()),
            "else the last line"
        );
        assert_eq!(fingerprint(""), None);
    }

    #[test]
    fn the_same_failure_on_two_heads_trips_b2_and_three_trips_b1() {
        let tail = tails("clippy", "error: unused variable `n` at src/a.rs:3");
        let failed = [("clippy", JobStatus::Failed, 1)];
        let mut b = Breaker::default();
        b = observe(&b, &change("a", State::Failed, &failed), &tail, 0);
        assert_eq!(
            signals(&b, 0, &Limits::default()),
            Vec::<Trip>::new(),
            "one head is no loop"
        );
        b = observe(&b, &change("b", State::Failed, &failed), &tail, 10);
        assert_eq!(names(&signals(&b, 10, &Limits::default())), ["B2"]);
        b = observe(&b, &change("c", State::Failed, &failed), &tail, 20);
        assert_eq!(names(&signals(&b, 20, &Limits::default())), ["B1", "B2"]);
    }

    #[test]
    fn a_different_failure_each_time_is_progress() {
        let failed = [("test", JobStatus::Failed, 1)];
        let mut b = Breaker::default();
        for (i, sha) in ["a", "b"].iter().enumerate() {
            let tail = tails("test", &format!("error: case {sha} differs"));
            b = observe(&b, &change(sha, State::Failed, &failed), &tail, i as u64);
        }
        assert_eq!(signals(&b, 2, &Limits::default()), Vec::<Trip>::new());
    }

    #[test]
    fn three_refusals_by_one_policy_trip_b3() {
        let mut b = Breaker::default();
        for (i, sha) in ["a", "b", "c"].iter().enumerate() {
            let mut c = change(sha, State::Refused, &[]);
            c.violations = vec![format!("outside its scope: src/{sha}.rs")];
            b = observe(&b, &c, &BTreeMap::new(), i as u64);
        }
        assert_eq!(names(&signals(&b, 3, &Limits::default())), ["B3"]);
    }

    #[test]
    fn eight_unchecked_heads_trip_b4_and_a_checked_one_resets_it() {
        let mut b = Breaker::default();
        for i in 0..UNCHECKED {
            let sha = format!("h{i}");
            b = observe(&b, &change(&sha, State::Bounded, &[]), &BTreeMap::new(), 0);
        }
        assert!(names(&signals(&b, 0, &Limits::default())).contains(&"B4"));
        b = observe(&b, &change("h7", State::Checked, &[]), &BTreeMap::new(), 0);
        assert!(!names(&signals(&b, 0, &Limits::default())).contains(&"B4"));
    }

    #[test]
    fn a_job_run_three_times_on_one_head_trips_b5() {
        let b = observe(
            &Breaker::default(),
            &change("a", State::Bounded, &[("test", JobStatus::Running, 3)]),
            &BTreeMap::new(),
            0,
        );
        assert_eq!(names(&signals(&b, 0, &Limits::default())), ["B5"]);
        let mirror = observe(
            &Breaker::default(),
            &change("a", State::Bounded, &[(MIRROR, JobStatus::Running, 3)]),
            &BTreeMap::new(),
            0,
        );
        assert!(
            signals(&mirror, 0, &Limits::default()).is_empty(),
            "the mirror isn't the change's"
        );
    }

    #[test]
    fn three_hours_over_three_unchecked_heads_trips_b6() {
        let mut b = Breaker::default();
        for (i, sha) in ["a", "b", "c"].iter().enumerate() {
            b = observe(
                &b,
                &change(sha, State::Bounded, &[]),
                &BTreeMap::new(),
                i as u64,
            );
        }
        assert_eq!(
            signals(&b, STUCK_FOR - 1, &Limits::default()),
            Vec::<Trip>::new()
        );
        assert_eq!(names(&signals(&b, STUCK_FOR, &Limits::default())), ["B6"]);
    }

    #[test]
    fn each_trip_is_reported_once() {
        let b = observe(
            &Breaker::default(),
            &change("a", State::Bounded, &[("test", JobStatus::Running, 3)]),
            &BTreeMap::new(),
            0,
        );
        let (b, first) = fresh(&b, 0, &Limits::default());
        assert_eq!(names(&first), ["B5"]);
        assert_eq!(fresh(&b, 0, &Limits::default()).1, Vec::<Trip>::new());
    }

    #[test]
    fn an_executors_change_trips_sooner_by_class() {
        let haiku = Limits::brief(Class::Haiku, Budget::default_for(Class::Haiku));
        let sonnet = Limits::brief(Class::Sonnet, Budget::default_for(Class::Sonnet));
        let failed = [("clippy", JobStatus::Failed, 1)];
        let mut b = Breaker::default();
        for (i, sha) in ["a", "b"].iter().enumerate() {
            let tail = tails("clippy", &format!("error: case {sha} differs"));
            b = observe(&b, &change(sha, State::Failed, &failed), &tail, i as u64);
        }
        assert_eq!(names(&signals(&b, 2, &haiku)), ["B1"]);
        assert_eq!(names(&signals(&b, 2, &sonnet)), ["B1"]);
        assert_eq!(
            signals(&b, 2, &Limits::default()),
            Vec::<Trip>::new(),
            "opus takes 3"
        );
        b = observe(&b, &change("c", State::Bounded, &[]), &BTreeMap::new(), 3);
        assert!(names(&signals(&b, 3, &haiku)).contains(&"B4"));
        assert!(!names(&signals(&b, 3, &sonnet)).contains(&"B4"));
        b = observe(&b, &change("d", State::Bounded, &[]), &BTreeMap::new(), 4);
        assert!(names(&signals(&b, 4, &sonnet)).contains(&"B4"));
    }

    #[test]
    fn a_briefs_budget_trips_past_its_heads_or_its_hours() {
        let budget = Budget { heads: 3, hours: 2 };
        let limits = Limits {
            budget: Some(budget),
            ..Limits::default()
        };
        let mut b = Breaker::default();
        for (i, sha) in ["a", "b", "c"].iter().enumerate() {
            b = observe(
                &b,
                &change(sha, State::Checked, &[]),
                &BTreeMap::new(),
                i as u64,
            );
        }
        assert_eq!(
            signals(&b, 3, &limits),
            Vec::<Trip>::new(),
            "three heads are within it"
        );
        b = observe(&b, &change("d", State::Checked, &[]), &BTreeMap::new(), 4);
        assert_eq!(names(&signals(&b, 4, &limits)), ["budget"]);

        let mut slow = Breaker::default();
        slow = observe(
            &slow,
            &change("a", State::Bounded, &[]),
            &BTreeMap::new(),
            0,
        );
        assert_eq!(signals(&slow, 2 * 3600 - 1, &limits), Vec::<Trip>::new());
        assert_eq!(names(&signals(&slow, 2 * 3600, &limits)), ["budget"]);
        slow = observe(
            &slow,
            &change("a", State::Checked, &[]),
            &BTreeMap::new(),
            2 * 3600,
        );
        assert_eq!(
            signals(&slow, 3 * 3600, &limits),
            Vec::<Trip>::new(),
            "a checked head waits on its planner, not its executor"
        );
    }
}
