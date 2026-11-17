//! The merge queue: one merge at a time per repository, in the order asked; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3. Each transition
//! answers the next queue; the `Merges` Durable Object stores it.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::types::{BOUND, Change, GATE, JobStatus, MERGE, MIRROR, RESTACK, State};

/// How long a merge may run before the queue moves past it, in seconds.
pub const STUCK: u64 = 60 * 60;

/// How long a turn is held before the queue asks its holder whether a merge is on its way, in
/// seconds; see the 2026-11-11 amendment on a turn held by no merge.
pub const SETTLE: u64 = 2 * 60;

/// Misses in a row after which a kept change's turn is protected; see the 2026-11-15 amendment
/// on a turn lost only because `main` moved.
pub const PROTECT: u32 = 2;

/// How long a protected turn holds the queue for its change, in seconds.
pub const HOLD: u64 = 20 * 60;

/// Seconds a release target waits for an idle queue before it takes the next turn anyway.
pub const RELEASE_WAIT: u64 = 30 * 60;

/// How long a change that gave its place back may take it again, in seconds; see the merge-path
/// ADR §8.
pub const RETURN: u64 = 60 * 60;

/// The one check a docs-only change runs.
const POLICY_CHECK: &str = "policy.check";

/// Whether change `c` is docs-only: its bound scope selects no code check, only `policy.check`;
/// see the merge-path ADR's 2026-11-17 amendment, §9.
#[must_use]
pub fn docs_only(c: &Change) -> bool {
    let plumbing = |n: &str| [BOUND, MIRROR, RESTACK, MERGE].contains(&n);
    c.jobs.iter().any(|j| j.name == POLICY_CHECK)
        && c.jobs
            .iter()
            .all(|j| j.name == POLICY_CHECK || plumbing(&j.name))
}

/// The change a yield names while its change's `ios` check waits for a macOS machine; no change
/// has this number. See the merge-path ADR §7.
pub const MACHINE: u32 = 0;

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

/// Whether change `c`, giving its turn back, keeps its place: it yields, and can still merge; see
/// the concurrent-sessions ADR's 2026-11-16 amendment.
#[must_use]
pub fn keeps_place(c: &Change) -> bool {
    c.yielded.is_some()
        && !matches!(
            c.state,
            State::Failed | State::Refused | State::Closed | State::Merged
        )
}

/// Change `c`'s session's pushes when its head failed; none when it didn't fail.
#[must_use]
pub fn failed_at(c: &Change) -> Option<u32> {
    (c.state == State::Failed).then_some(c.pushes)
}

/// Who asked for `after`'s merge, when, saved over `before`, its approval was just carried to a
/// new head with no merge on its way or queued: the control plane asks it again; see
/// `.claude/decisions/2026-11-16-development-merge-path-measured-faster.md` §4.
#[must_use]
pub fn reask(before: Option<&Change>, after: &Change, q: &MergeQueue) -> Option<String> {
    let b = before.filter(|b| b.state != State::Approved)?;
    let (c, a) = (b.carried.as_ref()?, after.approval.as_ref()?);
    let carried = after.state == State::Approved && a.by == c.by && a.at == c.at;
    let free = !holds_turn(after)
        && super::stage::place(q, after.number).is_none()
        && after.yielded.is_none()
        && after.conflict.is_none();
    if carried && free {
        after.merge_asked.clone()
    } else {
        None
    }
}

/// Whether holder `c` may give its turn back while its `ios` check waits for a macOS machine: its
/// merge is parked, it yields to none, and its approval carries, so its merge parks again when it
/// takes the turn back. See the merge-path ADR §7.
#[must_use]
pub fn machine_yieldable(c: &Change) -> bool {
    c.merge_after.is_some() && c.yielded.is_none() && c.carried.is_some()
}

/// Whether `c` gave its turn back while its `ios` check waits for a macOS machine.
#[must_use]
pub fn yielded_for_machine(c: &Change) -> bool {
    c.yielded.as_ref().is_some_and(|y| y.to == MACHINE)
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
    /// Whether the change is docs-only, as [`docs_only`] found it when asked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub docs: bool,
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

/// A change that lost its turn only because `main` moved: it is first when asked again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    /// Turns lost to `main` moving in a row.
    pub misses: u32,
    /// When it last lost one.
    pub at: u64,
    /// Its turn waits for its merge to be asked again: nothing else starts meanwhile, for at
    /// most [`SETTLE`].
    #[serde(default)]
    pub reserved: bool,
}

/// What the queue keeps of a waiting or merging change, to give its place back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// The owner moved it to the front.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fronted: bool,
    /// Its head's own lines when its merge was asked, as [`Change::own`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
    /// It already took a kept place back; it is used once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub returned: bool,
}

/// What a merge asked brings: its head's own lines and its session's pushes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Back {
    /// Its head's own lines, as [`Change::own`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
    /// Its session's pushes, as [`Change::pushes`].
    #[serde(default)]
    pub pushes: u32,
}

impl Back {
    /// Change `c`'s.
    #[must_use]
    pub fn of(c: &Change) -> Self {
        Self {
            own: c.own.clone(),
            pushes: c.pushes,
        }
    }
}

/// A change that gave its place back, and what lets a re-push take it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Returning {
    /// The changes waiting ahead of it; none when it held the turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ahead: Vec<u32>,
    /// The owner had moved it to the front.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fronted: bool,
    /// Its head's own lines when its merge was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
    /// When its head failed: its session's pushes then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<u32>,
    /// When it left.
    pub at: u64,
}

impl Returning {
    /// Why a merge asked with `back` at `now` takes the place back: the first push since its head
    /// failed, or, when it didn't fail, the same own lines; none otherwise, or after [`RETURN`].
    #[must_use]
    pub fn returns(&self, back: &Back, now: u64) -> Option<&'static str> {
        if now.saturating_sub(self.at) >= RETURN {
            return None;
        }
        match self.failed {
            Some(p) => (back.pushes == p.saturating_add(1))
                .then_some("it is the first push since its head failed"),
            None => (back.own.is_some() && back.own == self.own)
                .then_some("its own lines are unchanged"),
        }
    }
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
    /// The changes that lost their turn only because `main` moved.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kept: BTreeMap<u32, Kept>,
    /// When the release bot took the turn, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub releasing: Option<u64>,
    /// When the release bot asked for the next turn while it was busy, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_asked: Option<u64>,
    /// The docs-only changes still to merge in the docs-only turn running, in queue order; see
    /// the merge-path ADR's 2026-11-17 amendment, §9.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs_turn: Vec<u32>,
    /// Each waiting or merging change's [`Mark`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub marks: BTreeMap<u32, Mark>,
    /// The changes that gave their place back, each as [`Returning`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub returning: BTreeMap<u32, Returning>,
    /// The Instance's yields, each change to the one it waits for: read in, never stored; see
    /// the yield ADR's 2026-11-13 amendment on the queue keeping a yielded change's place.
    #[serde(skip)]
    pub yielded: BTreeMap<u32, u32>,
}

impl MergeQueue {
    /// Whether the release bot holds the turn at now, or has the next one reserved.
    #[must_use]
    pub fn release_holds(&self, now: u64) -> bool {
        self.releasing
            .is_some_and(|at| now.saturating_sub(at) < HOLD)
            || self
                .release_asked
                .is_some_and(|at| now.saturating_sub(at) < SETTLE)
    }

    /// The change holding the turn, once it has held it [`SETTLE`].
    #[must_use]
    pub fn holder(&self, now: u64) -> Option<u32> {
        self.merging
            .as_ref()
            .filter(|e| now.saturating_sub(e.at) >= SETTLE)
            .map(|e| e.number)
    }

    /// The kept change whose turn holds the queue at `now`, not yet asked again: reserved less
    /// than [`SETTLE`] ago, or protected, having missed [`PROTECT`] turns in a row less than
    /// [`HOLD`] ago.
    #[must_use]
    pub fn protected(&self, now: u64) -> Option<u32> {
        let asked = |n: u32| {
            self.merging.as_ref().is_some_and(|e| e.number == n)
                || self.waiting.iter().any(|e| e.number == n)
        };
        let holds = |k: &Kept| {
            let since = now.saturating_sub(k.at);
            (k.reserved && since < SETTLE) || (k.misses >= PROTECT && since < HOLD)
        };
        self.kept
            .iter()
            .find(|(n, k)| holds(k) && !asked(**n))
            .map(|(n, _)| *n)
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
    enqueue_as(q, number, by, false, now)
}

/// [`enqueue`], for a merge that brings `back`: a change that gave its place back, and
/// [`Returning::returns`] it, takes that place again, once, or the front behind any kept change
/// when the owner had moved it there; see the merge-path ADR §8.
///
/// # Errors
///
/// When it is already merging or waiting; the queue is then unchanged.
pub fn enqueue_back(
    q: &MergeQueue,
    number: u32,
    by: &str,
    back: &Back,
    now: u64,
) -> Result<(MergeQueue, Queued), String> {
    let (mut next, queued) = enqueue(q, number, by, now)?;
    let returning = q
        .returning
        .get(&number)
        .filter(|r| r.returns(back, now).is_some())
        .cloned();
    next.returning
        .retain(|_, r| now.saturating_sub(r.at) < RETURN);
    let fronted = returning.as_ref().is_some_and(|r| r.fronted)
        || q.marks.get(&number).is_some_and(|m| m.fronted);
    let mark = Mark {
        fronted,
        own: back.own.clone(),
        returned: q.marks.get(&number).is_some_and(|m| m.returned),
    };
    next.marks.insert(number, mark);
    let Some(r) = returning else {
        return Ok((next, queued));
    };
    if !matches!(queued, Queued::Behind(_)) || q.kept.contains_key(&number) {
        return Ok((next, queued));
    }
    let Some(entry) = next
        .waiting
        .iter()
        .position(|e| e.number == number)
        .and_then(|i| next.waiting.remove(i))
    else {
        return Ok((next, queued));
    };
    let kept = next
        .waiting
        .iter()
        .take_while(|e| next.kept.contains_key(&e.number))
        .count();
    let after = next
        .waiting
        .iter()
        .rposition(|e| r.ahead.contains(&e.number))
        .map_or(0, |i| i + 1);
    let at = if r.fronted { kept } else { after.max(kept) };
    next.waiting.insert(at, entry);
    next.returning.remove(&number);
    if let Some(m) = next.marks.get_mut(&number) {
        m.returned = true;
    }
    Ok((next, Queued::Behind(at + 1)))
}

/// As [`enqueue`], for a change that is docs-only when `docs`.
///
/// # Errors
///
/// As [`enqueue`].
pub fn enqueue_as(
    q: &MergeQueue,
    number: u32,
    by: &str,
    docs: bool,
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
        docs,
    };
    let mut next = q.clone();
    if let Some(&to) = q.yielded.get(&number) {
        next.waiting.push_back(entry);
        let place = next.waiting.len() + usize::from(next.merging.is_some());
        return Ok((next, Queued::Yielded { place, to }));
    }
    let kept = q.kept.contains_key(&number);
    let held = q.protected(now).is_some_and(|n| n != number) || q.release_holds(now);
    if let Some(k) = next.kept.get_mut(&number) {
        k.reserved = false;
    }
    if next.merging.is_none() && !held {
        next.docs_turn = docs_turn_of(&next, &entry);
        next.merging = Some(entry);
        return Ok((next, Queued::Start));
    }
    if kept {
        next.waiting.push_front(entry);
        return Ok((next, Queued::Behind(1)));
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
    ask_as(q, number, by, false, now)
}

/// [`ask`], for a merge that brings `back`, as [`enqueue_back`].
pub fn ask_back(
    q: &MergeQueue,
    number: u32,
    by: &str,
    back: &Back,
    now: u64,
) -> (MergeQueue, Option<Entry>, Result<Queued, String>) {
    let (q, resumed) = unstick(q, now);
    match enqueue_back(&q, number, by, back, now) {
        Ok((next, queued)) => (next, resumed, Ok(queued)),
        Err(why) => (q, resumed, Err(why)),
    }
}

/// As [`ask`], for a change that is docs-only when `docs`.
pub fn ask_as(
    q: &MergeQueue,
    number: u32,
    by: &str,
    docs: bool,
    now: u64,
) -> (MergeQueue, Option<Entry>, Result<Queued, String>) {
    let (q, resumed) = unstick(q, now);
    match enqueue_as(&q, number, by, docs, now) {
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
    next.kept.remove(&number);
    next.marks.remove(&number);
    next.docs_turn.retain(|n| *n != number);
    if next.merging.as_ref().is_some_and(|e| e.number == number) {
        next.merging = None;
    }
    advance(next, now)
}

/// Change `number` gave its place back without merging, its head failed at `failed` pushes or
/// not: the next queue, which keeps its place as [`Returning`], and the next merge to start, if
/// any, unless it already took a kept place back; see the merge-path ADR
/// §8.
#[must_use]
pub fn left(
    q: &MergeQueue,
    number: u32,
    failed: Option<u32>,
    now: u64,
) -> (MergeQueue, Option<Entry>) {
    let holding = q.merging.as_ref().is_some_and(|e| e.number == number);
    let waiting = q.waiting.iter().position(|e| e.number == number);
    let mut next = q.clone();
    next.returning
        .retain(|_, r| now.saturating_sub(r.at) < RETURN);
    let mark = q.marks.get(&number).cloned().unwrap_or_default();
    if (holding || waiting.is_some()) && !mark.returned {
        let returning = Returning {
            ahead: q
                .waiting
                .iter()
                .take(waiting.unwrap_or(0))
                .map(|e| e.number)
                .collect(),
            fronted: mark.fronted,
            own: mark.own,
            failed,
            at: now,
        };
        next.returning.insert(number, returning);
    }
    done(&next, number, now)
}

/// Change `number` yielded while holding or waiting for the turn: the next queue, with it first in
/// the waiting line, passed over until its yield ends, and the next merge to start, if any; see
/// the concurrent-sessions ADR's 2026-11-16 amendment.
#[must_use]
pub fn yielded_back(q: &MergeQueue, number: u32, now: u64) -> (MergeQueue, Option<Entry>) {
    let mut next = q.clone();
    let held = next.merging.take_if(|e| e.number == number);
    let waiting = next
        .waiting
        .iter()
        .position(|e| e.number == number)
        .and_then(|i| next.waiting.remove(i));
    if let Some(e) = held.or(waiting) {
        next.waiting.push_front(e);
    }
    next.kept.remove(&number);
    advance(next, now)
}

/// Change `number` lost its turn only because `main` moved: the next queue, with its turn taken
/// back and nothing started, so its merge asked again finds the turn free; see the 2026-11-15
/// amendment on a turn lost only because `main` moved.
#[must_use]
pub fn keep(q: &MergeQueue, number: u32, now: u64) -> MergeQueue {
    let mut next = q.clone();
    next.waiting.retain(|e| e.number != number);
    if next.merging.as_ref().is_some_and(|e| e.number == number) {
        next.merging = None;
    }
    let misses = next.kept.get(&number).map_or(0, |k| k.misses) + 1;
    let kept = Kept {
        misses,
        at: now,
        reserved: true,
    };
    next.kept.insert(number, kept);
    next
}

/// The owner moves waiting change `number` to the front of the line, behind the merge running
/// and any kept change: the next queue, and the place it left, as `(place, of)`. See the
/// concurrent-sessions ADR's 2026-11-15 amendment on the owner moving a change to the front.
///
/// # Errors
///
/// When it is merging, isn't queued, or yields to another; the queue is then unchanged.
pub fn front(q: &MergeQueue, number: u32) -> Result<(MergeQueue, (usize, usize)), String> {
    place(q, number, false)
}

/// A session moves its own waiting change `number` forward under Auto: as [`front`], but behind
/// every change already fronted; see
/// `.claude/decisions/2026-11-17-security-auto-covers-routine-owner-asks.md` §2.
///
/// # Errors
///
/// As [`front`].
pub fn front_after(q: &MergeQueue, number: u32) -> Result<(MergeQueue, (usize, usize)), String> {
    place(q, number, true)
}

fn place(
    q: &MergeQueue,
    number: u32,
    after_fronted: bool,
) -> Result<(MergeQueue, (usize, usize)), String> {
    if q.merging.as_ref().is_some_and(|e| e.number == number) {
        return Err(format!("change {number} is already being merged"));
    }
    if let Some(to) = q.yielded.get(&number) {
        return Err(format!(
            "change {number} yields to !{to}; its yield decides its place"
        ));
    }
    let Some(i) = q.waiting.iter().position(|e| e.number == number) else {
        return Err(format!(
            "change {number} isn't in the merge queue; ask to merge it first"
        ));
    };
    let running = usize::from(q.merging.is_some());
    let left = (i + 1 + running, q.waiting.len() + running);
    let mut next = q.clone();
    let Some(entry) = next.waiting.remove(i) else {
        return Err(format!("change {number} isn't in the merge queue"));
    };
    let ahead = next
        .waiting
        .iter()
        .take_while(|e| {
            q.kept.contains_key(&e.number)
                || (after_fronted && q.marks.get(&e.number).is_some_and(|m| m.fronted))
        })
        .count();
    next.waiting.insert(ahead, entry);
    next.marks.entry(number).or_default().fronted = true;
    Ok((next, left))
}

/// Change `number`'s kept turn is no longer reserved, as when its merge asked again waits for
/// the owner: the next queue, and the next merge to start, if any; a protected turn still holds.
#[must_use]
pub fn release(q: &MergeQueue, number: u32, now: u64) -> (MergeQueue, Option<Entry>) {
    let mut next = q.clone();
    if let Some(k) = next.kept.get_mut(&number) {
        k.reserved = false;
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

/// The release bot asks for the turn at `now`, its target having waited `waited` seconds: the
/// next queue, and whether it holds the turn; see the merge-path ADR §1.
#[must_use]
pub fn release_turn(q: &MergeQueue, waited: u64, now: u64) -> (MergeQueue, bool) {
    let mut next = q.clone();
    if next
        .releasing
        .is_some_and(|at| now.saturating_sub(at) < HOLD)
    {
        return (next, true);
    }
    let free = next.merging.is_none() && next.protected(now).is_none();
    let idle = next
        .waiting
        .iter()
        .all(|e| next.yielded.contains_key(&e.number));
    let due = waited >= RELEASE_WAIT;
    if free && (idle || due) {
        next.releasing = Some(now);
        next.release_asked = None;
        return (next, true);
    }
    if due
        && next
            .release_asked
            .is_none_or(|at| now.saturating_sub(at) >= SETTLE)
    {
        next.release_asked = Some(now);
    }
    (next, false)
}

/// The release bot's turn ended: the next queue, and the next merge to start, if any.
#[must_use]
pub fn release_turn_done(q: &MergeQueue, now: u64) -> (MergeQueue, Option<Entry>) {
    let mut next = q.clone();
    next.releasing = None;
    next.release_asked = None;
    advance(next, now)
}

/// `q` with its oldest waiting merge that yields to none started when none runs and no kept
/// change's turn is [`MergeQueue::protected`], and that merge; a yielded one keeps its place. A
/// docs-only turn running starts its next docs-only change first; see the merge-path ADR's
/// 2026-11-17 amendment, §9.
#[must_use]
pub fn advance(mut q: MergeQueue, now: u64) -> (MergeQueue, Option<Entry>) {
    if q.merging.is_some() || q.protected(now).is_some() || q.release_holds(now) {
        return (q, None);
    }
    let in_turn = q
        .waiting
        .iter()
        .position(|e| q.docs_turn.contains(&e.number) && !q.yielded.contains_key(&e.number));
    let Some(i) = in_turn.or_else(|| {
        q.waiting
            .iter()
            .position(|e| !q.yielded.contains_key(&e.number))
    }) else {
        q.docs_turn.clear();
        return (q, None);
    };
    let Some(mut e) = q.waiting.remove(i) else {
        return (q, None);
    };
    if in_turn.is_some() {
        q.docs_turn.retain(|n| *n != e.number);
    } else {
        q.docs_turn = docs_turn_of(&q, &e);
    }
    e.at = now;
    q.merging = Some(e.clone());
    (q, Some(e))
}

/// The docs-only changes waiting in `q`, in queue order, that merge in the turn `e` takes: none
/// unless `e` is docs-only; a yielded one keeps its place.
fn docs_turn_of(q: &MergeQueue, e: &Entry) -> Vec<u32> {
    if !e.docs {
        return Vec::new();
    }
    q.waiting
        .iter()
        .filter(|w| w.docs && w.number != e.number && !q.yielded.contains_key(&w.number))
        .map(|w| w.number)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::types::Job;
    use super::*;

    /// A queue with code change 9 merging, then `waiting` asked in order, each docs-only or not.
    fn queue(waiting: &[(u32, bool)]) -> MergeQueue {
        let (mut q, _) =
            enqueue_as(&MergeQueue::default(), 9, "window:1", false, 0).expect("starts");
        for &(n, docs) in waiting {
            q = enqueue_as(&q, n, "window:1", docs, 1).expect("queues").0;
        }
        q
    }

    /// The change `done(n)` starts, and the queue after it.
    fn next(q: &MergeQueue, n: u32) -> (MergeQueue, Option<u32>) {
        let (q, e) = done(q, n, 10);
        (q, e.map(|e| e.number))
    }

    fn with_jobs(names: &[&str]) -> Change {
        let mut c = Change::propose(7, "x", "docs: x", "p", vec!["**".into()], "agent");
        c.jobs = names.iter().map(|n| Job::queued(n, Vec::new())).collect();
        c
    }

    #[test]
    fn a_docs_only_change_runs_policy_check_alone() {
        assert!(docs_only(&with_jobs(&[BOUND, "policy.check"])));
        assert!(docs_only(&with_jobs(&[
            BOUND,
            "policy.check",
            MIRROR,
            RESTACK,
            MERGE
        ])));
        assert!(!docs_only(&with_jobs(&[BOUND, "fmt", "policy.check"])));
        assert!(!docs_only(&with_jobs(&[
            BOUND,
            "sites.build",
            "policy.check"
        ])));
        assert!(!docs_only(&with_jobs(&[
            BOUND,
            "policy.check",
            "gate: test"
        ])));
        assert!(!docs_only(&with_jobs(&[BOUND])), "not yet bound");
    }

    #[test]
    fn a_docs_turn_merges_every_waiting_docs_change_in_queue_order() {
        let q = queue(&[(1, true), (2, false), (3, true), (4, true)]);
        let (q, started) = next(&q, 9);
        assert_eq!(started, Some(1));
        assert_eq!(q.docs_turn, [3, 4]);
        let (q, started) = next(&q, 1);
        assert_eq!(started, Some(3), "it passes code change 2");
        let (q, queued) = enqueue_as(&q, 5, "window:1", true, 11).expect("queues");
        assert_eq!(queued, Queued::Behind(3));
        let (q, started) = next(&q, 3);
        assert_eq!(started, Some(4));
        let (q, started) = next(&q, 4);
        assert_eq!(
            started,
            Some(2),
            "a docs change asked during the turn waits its place"
        );
        assert_eq!(q.docs_turn, Vec::<u32>::new());
        let (_, started) = next(&q, 2);
        assert_eq!(started, Some(5));
    }

    #[test]
    fn a_failed_docs_change_drops_alone_and_the_turn_goes_on() {
        let q = queue(&[(1, true), (2, false), (3, true), (4, true)]);
        let (q, _) = next(&q, 9);
        let (q, _) = done(&q, 3, 10);
        assert_eq!(q.docs_turn, [4], "3 failed while it waited");
        let (q, started) = next(&q, 1);
        assert_eq!(started, Some(4));
        let (_, started) = next(&q, 4);
        assert_eq!(started, Some(2));
    }

    #[test]
    fn a_yielded_docs_change_keeps_its_place_outside_the_turn() {
        let mut q = queue(&[(1, true), (2, false), (3, true)]);
        q.yielded.insert(3, 2);
        let (q, started) = next(&q, 9);
        assert_eq!(started, Some(1));
        assert_eq!(q.docs_turn, Vec::<u32>::new());
        let (q, started) = next(&q, 1);
        assert_eq!(started, Some(2));
        assert_eq!(q.waiting.front().map(|e| e.number), Some(3));
    }

    #[test]
    fn a_code_turn_takes_no_docs_change_with_it() {
        let q = queue(&[(2, false), (1, true), (3, false)]);
        let (q, started) = next(&q, 9);
        assert_eq!(started, Some(2));
        assert_eq!(q.docs_turn, Vec::<u32>::new());
        let (_, started) = next(&q, 2);
        assert_eq!(started, Some(1));
    }

    #[test]
    fn an_entry_stored_before_docs_reads_as_not_docs() {
        let e: Entry = serde_json::from_str(r#"{"number":1,"by":"a","at":0}"#).expect("reads");
        assert!(!e.docs);
        let shown = serde_json::to_string(&e).expect("writes");
        assert_eq!(shown, r#"{"number":1,"by":"a","at":0}"#);
        let q: MergeQueue = serde_json::from_str("{}").expect("reads");
        assert_eq!(q.docs_turn, Vec::<u32>::new());
    }
}
