//! The merge queue: one merge at a time per repository, in the order asked; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3. Each transition
//! answers the next queue; the `Merges` Durable Object stores it.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::types::{Change, GATE, JobStatus, MERGE, RESTACK};

/// How long a merge may run before the queue moves past it, in seconds.
pub const STUCK: u64 = 60 * 60;

/// How long a turn is held before the queue asks its holder whether a merge is on its way, in
/// seconds; see the 2026-11-11 amendment on a turn held by no merge.
pub const SETTLE: u64 = 2 * 60;

/// Whether change `c` still has a merge on its way: a merge job, a restack or a gate check
/// queued or running, or a merge parked.
#[must_use]
pub fn holds_turn(c: &Change) -> bool {
    let on_its_way = |name: &str| name == MERGE || name == RESTACK || name.starts_with(GATE);
    c.merge_after.is_some()
        || c.jobs.iter().any(|j| {
            on_its_way(&j.name) && matches!(j.status, JobStatus::Queued | JobStatus::Running)
        })
}

/// Whether `from` yielding to `to` closes a cycle in `yields`: `to` already waits on `from`,
/// directly or through others, so neither would ever merge.
#[must_use]
pub fn closes_cycle(yields: &BTreeMap<u32, u32>, from: u32, to: u32) -> bool {
    let mut at = to;
    for _ in 0..=yields.len() {
        if at == from {
            return true;
        }
        match yields.get(&at) {
            Some(&next) => at = next,
            None => return false,
        }
    }
    true
}

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
    /// It keeps this place, passed over until its yield to `to` ends.
    Yielded {
        /// Its place.
        place: usize,
        /// The change it yields to.
        to: u32,
    },
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
    /// The Instance's yields, each change to the one it waits for: read in, never stored; see
    /// the yield ADR's 2026-11-13 amendment on the queue keeping a yielded change's place.
    #[serde(skip)]
    pub yielded: BTreeMap<u32, u32>,
}

impl MergeQueue {
    /// The change holding the turn, once it has held it [`SETTLE`].
    #[must_use]
    pub fn holder(&self, now: u64) -> Option<u32> {
        self.merging
            .as_ref()
            .filter(|e| now.saturating_sub(e.at) >= SETTLE)
            .map(|e| e.number)
    }
}

/// Asks for change `number`'s merge for `by` at `now`: the next queue, and where it stands.
///
/// # Errors
///
/// When it is already merging or waiting; the queue is then unchanged.
pub fn enqueue(
    q: &MergeQueue,
    number: u32,
    by: &str,
    now: u64,
) -> Result<(MergeQueue, Queued), String> {
    if q.merging.as_ref().is_some_and(|e| e.number == number) {
        return Err(format!("change {number} is already being merged"));
    }
    if let Some(i) = q.waiting.iter().position(|e| e.number == number) {
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
    let mut next = q.clone();
    if let Some(&to) = q.yielded.get(&number) {
        next.waiting.push_back(entry);
        let place = next.waiting.len() + usize::from(next.merging.is_some());
        return Ok((next, Queued::Yielded { place, to }));
    }
    if next.merging.is_none() {
        next.merging = Some(entry);
        return Ok((next, Queued::Start));
    }
    next.waiting.push_back(entry);
    let behind = next.waiting.len();
    Ok((next, Queued::Behind(behind)))
}

/// Asks for change `number`'s merge for `by` at `now`, first moving past a stuck turn: the next
/// queue, the merge it moved on to, which must start whatever this ask is answered, and the
/// answer.
pub fn ask(
    q: &MergeQueue,
    number: u32,
    by: &str,
    now: u64,
) -> (MergeQueue, Option<Entry>, Result<Queued, String>) {
    let (q, resumed) = unstick(q, now);
    match enqueue(&q, number, by, now) {
        Ok((next, queued)) => (next, resumed, Ok(queued)),
        Err(why) => (q, resumed, Err(why)),
    }
}

/// Change `number`'s merge ended, or never started: the next queue, and the next merge to start,
/// if any.
#[must_use]
pub fn done(q: &MergeQueue, number: u32, now: u64) -> (MergeQueue, Option<Entry>) {
    let mut next = q.clone();
    next.waiting.retain(|e| e.number != number);
    if next.merging.as_ref().is_some_and(|e| e.number == number) {
        next.merging = None;
    }
    advance(next, now)
}

/// Moves past a merge running longer than [`STUCK`]: the next queue, and the next merge to
/// start, if any.
#[must_use]
pub fn unstick(q: &MergeQueue, now: u64) -> (MergeQueue, Option<Entry>) {
    let mut next = q.clone();
    if next
        .merging
        .as_ref()
        .is_some_and(|e| now.saturating_sub(e.at) > STUCK)
    {
        next.merging = None;
    }
    advance(next, now)
}

/// `q` with its oldest waiting merge that yields to none started when none runs, and that
/// merge; a yielded one keeps its place.
#[must_use]
pub fn advance(mut q: MergeQueue, now: u64) -> (MergeQueue, Option<Entry>) {
    if q.merging.is_some() {
        return (q, None);
    }
    let Some(i) = q
        .waiting
        .iter()
        .position(|e| !q.yielded.contains_key(&e.number))
    else {
        return (q, None);
    };
    let Some(mut e) = q.waiting.remove(i) else {
        return (q, None);
    };
    e.at = now;
    q.merging = Some(e.clone());
    (q, Some(e))
}
