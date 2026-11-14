//! Tasks: how a session learns it has work, such as a reconciliation to carry out; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.

use serde::{Deserialize, Serialize};

use crate::sessions::Class;

/// How long a task waits for its own session before any session may take it, in seconds.
pub const OWN: u64 = 30 * 60;

/// The longest `task.wait` waits, in seconds: under the 30-second cut, as `events.wait`.
pub const WAIT_MAX: u64 = 25;

/// One task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Its number.
    pub id: u32,
    /// What it is: `reconcile`, `review` or `restack-failed`.
    pub kind: String,
    /// The change to work on.
    pub change: u32,
    /// The change it came from, if another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// The session it is for first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// What to do, in a line.
    pub note: String,
    /// When it was made, in Unix seconds.
    pub created: u64,
    /// The session that took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    /// Whether it is done.
    #[serde(default)]
    pub done: bool,
    /// For a `brief` task, the brief's id; see
    /// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// For a `brief` task, the class it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Class>,
}

impl Task {
    /// Whether a session of `class` may take it at `now`: any class when it names none; its own
    /// class at once; a more capable one once it has waited [`OWN`]; a less capable one never.
    #[must_use]
    pub fn for_class(&self, class: Class, now: u64) -> bool {
        match self.model {
            None => true,
            Some(m) if class == m => true,
            Some(m) => class.covers(m) && now.saturating_sub(self.created) >= OWN,
        }
    }

    /// Whether `session` may see and take it at `now`: its own session at once, any session
    /// once it has waited [`OWN`], and the one that took it.
    #[must_use]
    pub fn for_session(&self, session: Option<&str>, now: u64) -> bool {
        if self.done {
            return false;
        }
        if let Some(by) = &self.claimed_by {
            return session == Some(by.as_str());
        }
        self.session.is_none()
            || self.session.as_deref() == session
            || now.saturating_sub(self.created) >= OWN
    }
}

/// The task kinds whose work ends with their change, which the hourly sweep closes; a
/// `resume` or `test-map-failed` task outlives its change, as a brief's tasks will.
pub const SWEPT: [&str; 5] = [
    "reconcile",
    "review",
    "restack-failed",
    "yield-ended",
    "handoff",
];

/// Every task of an instance; done ones are kept a day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tasks {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// The tasks, oldest first.
    #[serde(default)]
    pub list: Vec<Task>,
}

/// How long a done task is kept, in seconds.
pub const KEPT: u64 = 24 * 60 * 60;

impl Tasks {
    /// Adds a task, numbered next, unless an open one of `kind` on `change` exists: its id.
    pub fn add(
        &mut self,
        kind: &str,
        change: u32,
        parent: Option<u32>,
        session: Option<String>,
        note: &str,
        now: u64,
    ) -> u32 {
        self.list
            .retain(|t| !t.done || now.saturating_sub(t.created) < KEPT);
        if let Some(t) = self
            .list
            .iter()
            .find(|t| !t.done && t.kind == kind && t.change == change)
        {
            return t.id;
        }
        self.last += 1;
        self.list.push(Task {
            id: self.last,
            kind: kind.to_string(),
            change,
            parent,
            session,
            note: note.to_string(),
            created: now,
            claimed_by: None,
            done: false,
            brief: None,
            model: None,
        });
        self.last
    }

    /// Adds the `brief` task for brief `id`, for `model`, unless an open one exists: its id.
    pub fn add_brief(&mut self, id: &str, model: Class, note: &str, now: u64) -> u32 {
        if let Some(t) = self
            .list
            .iter()
            .find(|t| !t.done && t.kind == "brief" && t.brief.as_deref() == Some(id))
        {
            return t.id;
        }
        let n = self.add("brief", 0, None, None, note, now);
        if let Some(t) = self.list.iter_mut().find(|t| t.id == n) {
            t.brief = Some(id.to_string());
            t.model = Some(model);
        }
        n
    }

    /// The open tasks `session` may see at `now`, as an opus session.
    #[must_use]
    pub fn visible(&self, session: Option<&str>, now: u64) -> Vec<&Task> {
        self.visible_as(session, Class::Opus, now)
    }

    /// The open tasks `session`, of `class`, may see at `now`.
    #[must_use]
    pub fn visible_as(&self, session: Option<&str>, class: Class, now: u64) -> Vec<&Task> {
        self.list
            .iter()
            .filter(|t| {
                t.for_session(session, now) && (t.claimed_by.is_some() || t.for_class(class, now))
            })
            .collect()
    }

    /// `session` takes task `id`.
    ///
    /// # Errors
    ///
    /// When there is no such open task, it is another session's for now, or taken.
    pub fn claim(&mut self, id: u32, session: &str, now: u64) -> Result<Task, String> {
        self.claim_as(id, session, Class::Opus, now)
    }

    /// `session`, of `class`, takes task `id`.
    ///
    /// # Errors
    ///
    /// As [`Tasks::claim`], and when it is for another class.
    pub fn claim_as(
        &mut self,
        id: u32,
        session: &str,
        class: Class,
        now: u64,
    ) -> Result<Task, String> {
        let t = self
            .list
            .iter_mut()
            .find(|t| t.id == id && !t.done)
            .ok_or_else(|| format!("no open task {id}"))?;
        if session.trim().is_empty() {
            return Err("a task is taken by a named `session`".into());
        }
        if !t.for_session(Some(session), now) {
            return Err(match &t.claimed_by {
                Some(by) => format!("task {id} is taken by {by}"),
                None => format!("task {id} is its own session's for now"),
            });
        }
        if t.claimed_by.is_none() && !t.for_class(class, now) {
            let m = t.model.map_or("", Class::name);
            return Err(if class.covers(t.model.unwrap_or_default()) {
                format!("task {id} is for a {m} session for now")
            } else {
                format!(
                    "task {id} is for a {m} session; a {} session can't take it",
                    class.name()
                )
            });
        }
        t.claimed_by = Some(session.to_string());
        Ok(t.clone())
    }

    /// Marks task `id` done.
    ///
    /// # Errors
    ///
    /// When there is no such task.
    pub fn done(&mut self, id: u32) -> Result<Task, String> {
        let t = self
            .list
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| format!("no task {id}"))?;
        t.done = true;
        Ok(t.clone())
    }

    /// Marks every open task on change `n` done, as when it merges: their ids.
    pub fn close_for(&mut self, n: u32) -> Vec<u32> {
        self.list
            .iter_mut()
            .filter(|t| !t.done && t.change == n)
            .map(|t| {
                t.done = true;
                t.id
            })
            .collect()
    }

    /// Marks every open task of a [`SWEPT`] kind done whose change isn't in `open`: their ids.
    pub fn sweep(&mut self, open: &std::collections::BTreeSet<u32>) -> Vec<u32> {
        self.list
            .iter_mut()
            .filter(|t| !t.done && SWEPT.contains(&t.kind.as_str()) && !open.contains(&t.change))
            .map(|t| {
                t.done = true;
                t.id
            })
            .collect()
    }
}

/// How long a task waits unclaimed before the owner's digest names it, in seconds; see
/// `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §4.
pub const STALE: u64 = 2 * 60 * 60;

/// What the owner's digest is built from.
#[derive(Debug, Clone, Default)]
pub struct DigestInput<'a> {
    /// Approvals waiting.
    pub waiting: &'a [crate::approve::Waiting],
    /// Every task.
    pub tasks: Option<&'a Tasks>,
    /// Every mandate.
    pub mandates: &'a [crate::mandates::Mandate],
    /// Deploys that failed since the last digest: the commit and why.
    pub failed_deploys: &'a [(String, String)],
    /// Changes merged without the owner since the last digest: the change and who approved.
    pub merged_without_owner: &'a [(u32, String)],
    /// Why the GitHub mirror needs the owner, if it does.
    pub mirror: Option<&'a str>,
    /// macOS jobs moved, providers down, and providers near their monthly cap; see
    /// `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §5.
    pub macos: &'a [String],
}

/// The owner's digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    /// Approvals waiting, oldest first: the change, the operation and the page.
    pub approvals: Vec<(u32, String, String)>,
    /// Tasks unclaimed longer than [`STALE`].
    pub stale_tasks: Vec<Task>,
    /// Mandates paused, with why.
    pub paused: Vec<(u32, String)>,
    /// Deploys that failed: the commit and why.
    pub failed_deploys: Vec<(String, String)>,
    /// Changes merged without the owner: the change and who approved.
    pub merged_without_owner: Vec<(u32, String)>,
    /// Why the GitHub mirror needs the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    /// Each session with open changes: shown, never pushed; see
    /// `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §5.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionLine>,
    /// Changes that need a session that isn't listening: pushed, as nothing automatic moves
    /// them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stalled: Vec<Stall>,
    /// macOS jobs moved, providers down, and providers near their cap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macos: Vec<String>,
}

/// How long a session goes without polling before a change that needs it stalls, in seconds;
/// see the session-stages ADR's 2026-11-13 amendment on stalls.
pub const STALL: u64 = 30 * 60;

/// How long `polled`'s session has been quiet at `now`, when a change of `needs` stalls on it:
/// `Some(None)` for a session that never polled.
#[must_use]
#[allow(clippy::option_option)]
pub fn stalled(polled: Option<u64>, needs: &str, now: u64) -> Option<Option<u64>> {
    if needs != "agent" {
        return None;
    }
    match polled {
        None => Some(None),
        Some(at) => (now.saturating_sub(at) >= STALL).then(|| Some(now.saturating_sub(at))),
    }
}

/// A change that needs its session, which isn't listening.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stall {
    /// The change.
    pub number: u32,
    /// Its session's label.
    pub session: String,
    /// Its stage.
    pub stage: String,
    /// What its session must do.
    pub next: String,
    /// How long its session has been quiet, in seconds; none if it never polled.
    pub quiet: Option<u64>,
    /// The open changes yielded to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<u32>,
}

impl Stall {
    /// The line the digest shows, and pushes.
    #[must_use]
    pub fn line(&self) -> String {
        let short = self.session.rsplit('/').next().unwrap_or(&self.session);
        let quiet = self.quiet.map_or("never polled".into(), |q| {
            format!("quiet {}h{:02}m", q / 3600, q % 3600 / 60)
        });
        let waiting: Vec<String> = self.waiting.iter().map(|n| format!("!{n}")).collect();
        let behind = if waiting.is_empty() {
            String::new()
        } else {
            let verb = if waiting.len() == 1 { "waits" } else { "wait" };
            format!("; {} {verb} on it", waiting.join(", "))
        };
        format!(
            "stalled: !{} {} needs session {short} ({quiet}): {}{behind}; start a session to \
             take it over, or close it",
            self.number, self.stage, self.next
        )
    }
}

/// A session with open changes, as the digest shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLine {
    /// Its label, the session's URL.
    pub label: String,
    /// Whether it polled its events in the last two minutes.
    pub listening: bool,
    /// Its open changes: the number, the stage and who it needs.
    pub changes: Vec<(u32, String, String)>,
}

impl SessionLine {
    /// The line the digest shows: the label's last part, whether it listens, and its changes,
    /// those needing the agent first.
    #[must_use]
    pub fn line(&self) -> String {
        let short = self.label.rsplit('/').next().unwrap_or(&self.label);
        let mut changes = self.changes.clone();
        changes.sort_by_key(|(n, _, needs)| (needs != "agent", *n));
        let shown: Vec<String> = changes
            .iter()
            .map(|(n, stage, needs)| format!("!{n} {stage} ({needs})"))
            .collect();
        let ear = if self.listening {
            "listening"
        } else {
            "not listening"
        };
        format!("session {short}: {ear}; {}", shown.join(", "))
    }
}

impl Digest {
    /// Whether it holds something the owner must act on, so it is pushed.
    #[must_use]
    pub fn needs_owner(&self) -> bool {
        !(self.approvals.is_empty()
            && self.stale_tasks.is_empty()
            && self.paused.is_empty()
            && self.failed_deploys.is_empty()
            && self.stalled.is_empty()
            && self.mirror.is_none())
    }

    /// Whether it holds anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.needs_owner() && self.merged_without_owner.is_empty() && self.macos.is_empty()
    }

    /// One line per item, for a page: [`Self::owner_lines`], then the sessions.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = self.owner_lines();
        out.extend(self.sessions.iter().map(SessionLine::line));
        out
    }

    /// One line per item for the owner, for a push: everything but the sessions.
    #[must_use]
    pub fn owner_lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .approvals
            .iter()
            .map(|(n, op, path)| format!("waiting: `{op}` on change {n}: {path}"))
            .collect();
        out.extend(self.stale_tasks.iter().map(|t| {
            format!(
                "unclaimed: task {} ({}) on change {}: {}",
                t.id, t.kind, t.change, t.note
            )
        }));
        out.extend(
            self.paused
                .iter()
                .map(|(id, why)| format!("paused: mandate {id}: {why}")),
        );
        out.extend(
            self.failed_deploys.iter().map(|(sha, why)| {
                format!("deploy failed: {}: {why}", sha.get(..12).unwrap_or(sha))
            }),
        );
        out.extend(
            self.merged_without_owner
                .iter()
                .map(|(n, by)| format!("merged without you: change {n}, by {by}")),
        );
        out.extend(self.stalled.iter().map(Stall::line));
        out.extend(self.mirror.clone());
        out.extend(self.macos.iter().map(|l| format!("macOS: {l}")));
        out
    }
}

/// The owner's digest at `now`.
#[must_use]
pub fn digest(input: &DigestInput<'_>, now: u64) -> Digest {
    let mut waiting: Vec<&crate::approve::Waiting> =
        input.waiting.iter().filter(|w| w.expires > now).collect();
    waiting.sort_by_key(|w| w.expires);
    Digest {
        approvals: waiting
            .iter()
            .map(|w| (w.number, w.operation.clone(), w.path.clone()))
            .collect(),
        stale_tasks: input
            .tasks
            .map(|t| {
                t.list
                    .iter()
                    .filter(|t| {
                        !t.done && t.claimed_by.is_none() && now.saturating_sub(t.created) >= STALE
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        paused: input
            .mandates
            .iter()
            .filter(|m| m.state == crate::mandates::State::Paused)
            .map(|m| (m.id, m.why.clone().unwrap_or_default()))
            .collect(),
        failed_deploys: input.failed_deploys.to_vec(),
        merged_without_owner: input.merged_without_owner.to_vec(),
        mirror: input.mirror.map(str::to_string),
        sessions: Vec::new(),
        stalled: Vec::new(),
        macos: input.macos.to_vec(),
    }
}
