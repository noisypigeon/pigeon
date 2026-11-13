//! The instance's event log: every completion that matters, which sessions follow instead of
//! polling; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §1.

use std::collections::VecDeque;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// The most events kept.
pub const KEEP: usize = 2000;

/// The longest an event is kept, in seconds.
pub const KEEP_FOR: u64 = 7 * 24 * 60 * 60;

/// The most events one read returns.
pub const PAGE: usize = 200;

/// One event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Its number, rising.
    pub id: u64,
    /// When, in Unix seconds.
    pub at: u64,
    /// What happened, such as `change.checked` or `deploy.failed`.
    pub kind: String,
    /// The change it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<u32>,
    /// That change's session, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// What happened, in a line.
    pub summary: String,
    /// Shown only to the owner, as an approval's link is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner: bool,
    /// The change's stage as the event was appended; see
    /// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<crate::core::change::stage::Stage>,
}

/// Which events a reader wants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only after this event.
    pub after: u64,
    /// Only this session's, and those of no session.
    pub session: Option<String>,
    /// Only these changes', when any are named.
    pub changes: Vec<u32>,
    /// The reader is the owner, so owner-only events are included.
    pub owner: bool,
}

impl Filter {
    fn wants(&self, e: &Event) -> bool {
        e.id > self.after
            && (self.owner || !e.owner)
            && (self.changes.is_empty() || e.change.is_some_and(|c| self.changes.contains(&c)))
            && match (&self.session, &e.session) {
                (Some(want), Some(got)) => want == got,
                _ => true,
            }
    }
}

/// The log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    /// The last number given.
    #[serde(default)]
    pub last: u64,
    /// The events kept, oldest first.
    #[serde(default)]
    pub list: VecDeque<Event>,
}

impl Log {
    /// Appends an event at `now`, dropping what is too old or too many: its number.
    pub fn append(
        &mut self,
        kind: &str,
        change: Option<u32>,
        session: Option<String>,
        summary: &str,
        owner: bool,
        now: u64,
    ) -> u64 {
        self.last += 1;
        self.list.push_back(Event {
            id: self.last,
            at: now,
            kind: kind.to_string(),
            change,
            session,
            summary: summary.to_string(),
            owner,
            stage: None,
        });
        while self.list.len() > KEEP
            || self
                .list
                .front()
                .is_some_and(|e| now.saturating_sub(e.at) > KEEP_FOR)
        {
            self.list.pop_front();
        }
        self.last
    }

    /// The events `f` wants, oldest first, at most [`PAGE`].
    #[must_use]
    pub fn since(&self, f: &Filter) -> Vec<&Event> {
        self.list.iter().filter(|e| f.wants(e)).take(PAGE).collect()
    }
}

/// What happened to `after` since `before`, as event kinds and summaries: its state, its jobs'
/// starts and ends, and its falling behind or into conflict.
#[must_use]
pub fn of_change(
    before: Option<&crate::changes::Change>,
    after: &crate::changes::Change,
) -> Vec<(String, String)> {
    use crate::changes::JobStatus;
    let n = after.number;
    let mut out = Vec::new();
    if before.is_none_or(|b| b.state != after.state) {
        let mut summary = format!("!{n} {}: {}", after.title, after.state.name());
        if let Some(a) = &after.approval
            && after.state == crate::changes::State::Approved
        {
            let _ = write!(summary, " by {}", a.by);
        }
        if let Some(v) = after.violations.first() {
            let _ = write!(summary, ": {v}");
        }
        out.push((format!("change.{}", after.state.name()), summary));
    }
    let was = |name: &str| {
        before
            .filter(|b| b.head == after.head)
            .and_then(|b| b.jobs.iter().find(|j| j.name == name))
            .map(|j| j.status)
    };
    for j in &after.jobs {
        let kind = match (was(&j.name), j.status) {
            (Some(JobStatus::Running), JobStatus::Running) => None,
            (_, JobStatus::Running) => Some("job.started"),
            (Some(s), JobStatus::Passed) if s != JobStatus::Passed => Some("job.passed"),
            (Some(s), JobStatus::Failed) if s != JobStatus::Failed => Some("job.failed"),
            _ => None,
        };
        if let Some(k) = kind {
            out.push((k.to_string(), format!("!{n} job `{}`", j.name)));
        }
    }
    if after.behind.is_some() && before.is_none_or(|b| b.behind != after.behind) {
        out.push(("change.behind".into(), format!("!{n} is behind main")));
    }
    if let Some(c) = &after.conflict
        && before.is_none_or(|b| b.conflict.as_ref().map(|x| &x.reconcile) != Some(&c.reconcile))
    {
        let by = c
            .reconcile
            .map(|r| format!("; !{r} reconciles it"))
            .unwrap_or_default();
        out.push((
            "change.conflicted".into(),
            format!("!{n} conflicts with main in {}{by}", c.paths.join(", ")),
        ));
    }
    out
}
