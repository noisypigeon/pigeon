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
    pub stage: Option<crate::change::stage::Stage>,
    /// Its session's model class as the event was appended; see
    /// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::sessions::Class>,
    /// What it is about across events, such as `owner:7` or `pool:anthropic`; see
    /// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md, step 8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The earlier event it closes, which the log sets; never a producer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolves: Option<u64>,
}

/// Kinds that open a subject.
pub const OPENS: &[&str] = &[
    "owner.asked",
    "grant.requested",
    "approval.pending",
    "pool.paused",
    "auto.paused",
    "auto.held",
    "task.made",
    "mandate.proposed",
];

/// Kinds that close one.
pub const CLOSES: &[&str] = &[
    "owner.answered",
    "owner.withdrawn",
    "grant.granted",
    "grant.failed",
    "grant.rotated",
    "grant.revoked",
    "pool.resumed",
    "auto.resumed",
    "auto.ended",
    "task.done",
    "mandate.granted",
    "mandate.ended",
    "ci.window",
];

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
    /// Only what the agent must act on ([`wakes`]); see
    /// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §2.
    pub wake: bool,
}

/// Kinds that need the agent whatever stage they carry.
const WAKING: &[&str] = &[
    "change.merged",
    "change.closed",
    "change.conflicted",
    "change.evicted",
    "change.handed",
    "change.note",
    "change.yielded",
    "change.unyielded",
    "merge.released",
    "task.made",
    "deploy.failed",
    "grant.failed",
    "auto.paused",
    "auto.held",
    "auto.resumed",
    "auto.ended",
    "session.replaced",
    "session.archivable",
    "owner.answered",
];

/// Whether `e` needs the agent: its change's stage does, or the change is done, closed, its
/// deploy failed or it was attended, or its kind is in [`WAKING`]; never a job's event, which
/// comes with its change's own.
#[must_use]
pub fn wakes(e: &Event) -> bool {
    use crate::change::stage::Needs;
    if e.kind.starts_with("job.") {
        return false;
    }
    e.stage.as_ref().is_some_and(|s| {
        s.needs == Needs::Agent
            || ["done", "closed", "deploy-failed", "attended"].contains(&s.stage.as_str())
    }) || WAKING.contains(&e.kind.as_str())
}

/// How long a change needs its session before it is nudged, in seconds; see the session-stages
/// ADR's 2026-11-14 amendment.
pub const NUDGE_AFTER: u64 = 15 * 60;

/// The least time between two nudges of one change, in seconds.
pub const NUDGE_EVERY: u64 = 60 * 60;

/// The kind of a nudge.
pub const NUDGED: &str = "change.nudged";

/// Whether an event of `kind` moves its change, for its stage's `since`: not a job's, nor a nudge.
#[must_use]
pub fn moves(kind: &str) -> bool {
    !kind.starts_with("job.") && kind != NUDGED
}

/// Whether the hourly run asks the owner's window to authorize `c`: a plan no one has approved,
/// whoever proposed it, the control plane's reviews and reconciliations too; see
/// `.claude/decisions/2026-11-15-development-sessions-stop-only-for-oversight.md`'s 2026-11-15
/// amendment on generated changes.
#[must_use]
pub fn authorize_due(c: &crate::change::Change) -> bool {
    c.state == crate::change::State::Proposed && c.plan_approval.is_none()
}

/// Whether a change at `stage` is due a nudge at `now`: it has needed its session for at least
/// [`NUDGE_AFTER`].
#[must_use]
pub fn due(stage: &crate::change::stage::Stage, now: u64) -> bool {
    stage.needs == crate::change::stage::Needs::Agent
        && stage
            .since
            .is_some_and(|at| now.saturating_sub(at) >= NUDGE_AFTER)
}

/// A nudge's summary: how long change `number` has needed its session, and what to do.
#[must_use]
pub fn nudge_summary(number: u32, stage: &crate::change::stage::Stage, now: u64) -> String {
    let waited = now.saturating_sub(stage.since.unwrap_or(now)) / 60;
    format!(
        "!{number} has needed its session for {}h{:02}m: {}",
        waited / 60,
        waited % 60,
        stage.next
    )
}

/// A long poll's next sleep, in milliseconds: at most `every`, never past `until`; `None` once
/// `now` reaches it. See `.claude/decisions/2026-11-13-development-rust-session-event-follower.md` §1.
#[must_use]
pub fn look(now: u64, until: u64, every: u64) -> Option<u64> {
    (now < until).then(|| every.min(until - now))
}

/// How long a session counts as listening after its last poll, in seconds.
pub const LISTENING: u64 = 2 * 60;

/// Where a session's reading of the log is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// The highest `after` it polled with: what it has seen.
    pub after: u64,
    /// When it last polled, in Unix seconds.
    pub polled: u64,
}

/// Where a poll starts: from the session's cursor, or a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// From the session's cursor.
    Session,
    /// After this event.
    At(u64),
}

/// Each session's cursor, by label; kept [`KEEP_FOR`] after its last poll.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursors(pub std::collections::BTreeMap<String, Cursor>);

impl Cursors {
    /// Records `label`'s poll at `now` and answers where it starts: its cursor for
    /// [`After::Session`] (the log's `last` for a session never seen), else the number given,
    /// which moves the cursor forward. The cursor never passes `last`.
    pub fn poll(&mut self, label: &str, asked: After, last: u64, now: u64) -> u64 {
        self.0
            .retain(|_, c| now.saturating_sub(c.polled) <= KEEP_FOR);
        let c = self.0.entry(label.to_string()).or_insert(Cursor {
            after: last,
            polled: now,
        });
        c.polled = now;
        c.after = c.after.min(last);
        match asked {
            After::Session => c.after,
            After::At(n) => {
                c.after = c.after.max(n.min(last));
                n
            }
        }
    }

    /// `label`'s cursor, and whether it polled within [`LISTENING`] of `now`.
    #[must_use]
    pub fn listening(&self, label: &str, now: u64) -> Option<(Cursor, bool)> {
        self.0
            .get(label)
            .map(|c| (*c, now.saturating_sub(c.polled) <= LISTENING))
    }
}

impl Filter {
    fn wants(&self, e: &Event) -> bool {
        e.id > self.after
            && (!self.wake || wakes(e))
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
            model: None,
            subject: None,
            resolves: None,
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

    /// Appends a nudge of `change` at `stage`, unless one was appended within [`NUDGE_EVERY`] of
    /// `now`: its number, if appended.
    pub fn nudge(
        &mut self,
        change: u32,
        session: Option<String>,
        stage: crate::change::stage::Stage,
        now: u64,
    ) -> Option<u64> {
        let recent = self.list.iter().any(|e| {
            e.kind == NUDGED && e.change == Some(change) && now.saturating_sub(e.at) < NUDGE_EVERY
        });
        if recent {
            return None;
        }
        let summary = nudge_summary(change, &stage, now);
        let id = self.append(NUDGED, Some(change), session, &summary, false, now);
        if let Some(e) = self.list.back_mut().filter(|e| e.id == id) {
            e.stage = Some(stage);
        }
        Some(id)
    }

    /// Names event `id`'s subject; a closing kind resolves the newest earlier open event of that
    /// subject that nothing resolves yet.
    pub fn link(&mut self, id: u64, subject: &str) {
        if subject.trim().is_empty() {
            return;
        }
        let Some(at) = self.list.iter().position(|e| e.id == id) else {
            return;
        };
        self.list[at].subject = Some(subject.to_string());
        if !CLOSES.contains(&self.list[at].kind.as_str()) {
            return;
        }
        let open = self.list.iter().take(at).rev().find(|e| {
            e.subject.as_deref() == Some(subject)
                && OPENS.contains(&e.kind.as_str())
                && !self.list.iter().any(|r| r.resolves == Some(e.id))
        });
        let found = open.map(|e| e.id);
        self.list[at].resolves = found;
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
    before: Option<&crate::change::Change>,
    after: &crate::change::Change,
) -> Vec<(String, String)> {
    use crate::change::JobStatus;
    let n = after.number;
    let mut out = Vec::new();
    if before.is_none_or(|b| b.state != after.state) {
        let mut summary = format!("!{n} {}: {}", after.title, after.state.name());
        if let Some(a) = &after.approval
            && after.state == crate::change::State::Approved
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
            (Some(s), JobStatus::Waived) if s != JobStatus::Waived => Some("job.waived"),
            _ => None,
        };
        if let Some(k) = kind {
            out.push((k.to_string(), format!("!{n} job `{}`", j.name)));
        }
        if let Some(r) = retrying(before, after, j) {
            out.push(("job.retrying".into(), r));
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

/// The `job.retrying` summary for job `j`, when it is queued again on the same head after a
/// failed start or a lost Container; see
/// `.claude/decisions/2026-11-15-development-job-start-failures-backoff.md` §3.
fn retrying(
    before: Option<&crate::change::Change>,
    after: &crate::change::Change,
    j: &crate::change::Job,
) -> Option<String> {
    use crate::change::JobStatus;
    let b = before
        .filter(|b| b.head == after.head)?
        .jobs
        .iter()
        .find(|x| x.name == j.name)?;
    if j.status != JobStatus::Queued {
        return None;
    }
    let why = j.last_error.as_deref().unwrap_or("no reason was given");
    let n = after.number;
    if j.start_failures > b.start_failures {
        let max = crate::change::MAX_START_FAILURES;
        let k = j.start_failures;
        return Some(format!(
            "!{n} job `{}` didn't start ({k} of {max}): {why}",
            j.name
        ));
    }
    (b.status == JobStatus::Running).then(|| {
        let a = j.attempts;
        let max = crate::platform::runs::MAX_ATTEMPTS;
        format!("!{n} job `{}`: {why} (attempt {a} of {max})", j.name)
    })
}

#[cfg(test)]
#[path = "events/link_tests.rs"]
mod link_tests;
