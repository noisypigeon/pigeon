//! The old in-place calls over `step` and the merge queue's transitions, so replay tests read as
//! before: a refusal leaves the value unchanged.
#![allow(dead_code)]

use backplane::changes::{self, Change, Effect, Event, Refusal};
use backplane::merges::{self, Entry, MergeQueue, Queued};

/// `Change` stepped in place.
pub trait Apply {
    /// Steps by `event` and keeps the next change.
    fn apply(&mut self, event: Event) -> Result<Vec<Effect>, Refusal>;
}

impl Apply for Change {
    fn apply(&mut self, event: Event) -> Result<Vec<Effect>, Refusal> {
        let (next, effects) = changes::step(self, event)?;
        *self = next;
        Ok(effects)
    }
}

/// `MergeQueue` stepped in place.
pub trait Queue {
    /// [`merges::enqueue`], kept.
    fn enqueue(&mut self, number: u32, by: &str, now: u64) -> Result<Queued, String>;
    /// [`merges::ask`], kept.
    fn ask(&mut self, number: u32, by: &str, now: u64) -> (Option<Entry>, Result<Queued, String>);
    /// [`merges::done`], kept.
    fn done(&mut self, number: u32, now: u64) -> Option<Entry>;
    /// [`merges::unstick`], kept.
    fn unstick(&mut self, now: u64) -> Option<Entry>;
}

impl Queue for MergeQueue {
    fn enqueue(&mut self, number: u32, by: &str, now: u64) -> Result<Queued, String> {
        let (next, queued) = merges::enqueue(self, number, by, now)?;
        *self = next;
        Ok(queued)
    }

    fn ask(&mut self, number: u32, by: &str, now: u64) -> (Option<Entry>, Result<Queued, String>) {
        let (next, resumed, queued) = merges::ask(self, number, by, now);
        *self = next;
        (resumed, queued)
    }

    fn done(&mut self, number: u32, now: u64) -> Option<Entry> {
        let (next, entry) = merges::done(self, number, now);
        *self = next;
        entry
    }

    fn unstick(&mut self, now: u64) -> Option<Entry> {
        let (next, entry) = merges::unstick(self, now);
        *self = next;
        entry
    }
}
