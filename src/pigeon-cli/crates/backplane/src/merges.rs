//! The merge queue: one merge at a time per repository, in the order asked; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// How long a merge may run before the queue moves past it, in seconds.
pub const STUCK: u64 = 60 * 60;

/// A merge asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The change.
    pub number: u32,
    /// Who asked.
    pub by: String,
    /// When it was asked for, or when its merge started.
    pub at: u64,
}

/// Where a merge asked for stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Queued {
    /// Nothing else merges: start it now.
    Start,
    /// It waits behind this many merges.
    Behind(usize),
}

/// One repository's merges: the one running and those waiting, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueue {
    /// The merge running, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merging: Option<Entry>,
    /// The merges waiting.
    #[serde(default)]
    pub waiting: VecDeque<Entry>,
}

impl MergeQueue {
    /// Asks for change `number`'s merge for `by` at `now`.
    ///
    /// # Errors
    ///
    /// When it is already merging or waiting.
    pub fn enqueue(&mut self, number: u32, by: &str, now: u64) -> Result<Queued, String> {
        if self.merging.as_ref().is_some_and(|e| e.number == number) {
            return Err(format!("change {number} is already being merged"));
        }
        if let Some(i) = self.waiting.iter().position(|e| e.number == number) {
            return Err(format!(
                "change {number} already waits behind {} merge(s)",
                i + 1
            ));
        }
        let entry = Entry {
            number,
            by: by.to_string(),
            at: now,
        };
        if self.merging.is_none() {
            self.merging = Some(entry);
            return Ok(Queued::Start);
        }
        self.waiting.push_back(entry);
        Ok(Queued::Behind(self.waiting.len()))
    }

    /// Change `number`'s merge ended, or never started: the next merge to start, if any.
    pub fn done(&mut self, number: u32, now: u64) -> Option<Entry> {
        self.waiting.retain(|e| e.number != number);
        if self.merging.as_ref().is_some_and(|e| e.number == number) {
            self.merging = None;
        }
        self.next(now)
    }

    /// Moves past a merge running longer than [`STUCK`]: the next merge to start, if any.
    pub fn unstick(&mut self, now: u64) -> Option<Entry> {
        if self
            .merging
            .as_ref()
            .is_some_and(|e| now.saturating_sub(e.at) > STUCK)
        {
            self.merging = None;
        }
        self.next(now)
    }

    fn next(&mut self, now: u64) -> Option<Entry> {
        if self.merging.is_some() {
            return None;
        }
        let mut e = self.waiting.pop_front()?;
        e.at = now;
        self.merging = Some(e.clone());
        Some(e)
    }
}
