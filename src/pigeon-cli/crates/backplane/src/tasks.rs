//! Tasks: how a session learns it has work, such as a reconciliation to carry out; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.

use serde::{Deserialize, Serialize};

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
}

impl Task {
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
        });
        self.last
    }

    /// The open tasks `session` may see at `now`.
    #[must_use]
    pub fn visible(&self, session: Option<&str>, now: u64) -> Vec<&Task> {
        self.list
            .iter()
            .filter(|t| t.for_session(session, now))
            .collect()
    }

    /// `session` takes task `id`.
    ///
    /// # Errors
    ///
    /// When there is no such open task, it is another session's for now, or taken.
    pub fn claim(&mut self, id: u32, session: &str, now: u64) -> Result<Task, String> {
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
}

impl Digest {
    /// Whether it holds something the owner must act on, so it is pushed.
    #[must_use]
    pub fn needs_owner(&self) -> bool {
        !(self.approvals.is_empty()
            && self.stale_tasks.is_empty()
            && self.paused.is_empty()
            && self.failed_deploys.is_empty()
            && self.mirror.is_none())
    }

    /// Whether it holds anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.needs_owner() && self.merged_without_owner.is_empty()
    }

    /// One line per item, for a push or a page.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
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
        out.extend(self.mirror.clone());
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
    }
}
