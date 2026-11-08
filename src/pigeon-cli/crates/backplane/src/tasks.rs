//! Tasks: how a session learns it has work, such as a reconciliation to carry out; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §5.

use serde::{Deserialize, Serialize};

/// How long a task waits for its own session before any session may take it, in seconds.
pub const OWN: u64 = 30 * 60;

/// The longest `task.wait` waits, in seconds.
pub const WAIT_MAX: u64 = 50;

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
