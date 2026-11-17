//! Auto: a standing window that lets every change at the code tier or lower through, the
//! gate (or, with no gate set, every guarded and eviction path) cut out, started and paused with
//! a passkey; see
//! `.claude/decisions/2026-11-15-security-standing-auto-owner-flags.md`.

use serde::{Deserialize, Serialize};

use super::mandates::{Decision, Hold, State, defers, fixes};
use super::tiers::{Tier, tier_of};

/// The merges in [`SPAN`] that hold Auto: the rate breaker.
pub const RATE: usize = 150;
/// How long without a counted merge before the rate breaker's hold releases, in seconds.
pub const QUIET: u64 = 3_600;
/// The rate breaker's span, in seconds.
pub const SPAN: u64 = 6 * 3_600;
/// The passkey purpose for starting a window.
pub const START: &str = "window:start";
/// The passkey purpose for ending one early.
pub const END: &str = "window:end";
/// How many ended windows are kept, newest last.
const KEEP: usize = 20;

/// One Auto window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// Its number.
    pub id: u32,
    /// `0`: a standing window; kept for older records.
    pub hours: u32,
    /// When it started, in Unix seconds.
    pub started: u64,
    /// `started` for a standing window; kept for older records.
    pub expires: u64,
    /// The merges made under it.
    #[serde(default)]
    pub used: u32,
    /// Where it is.
    pub state: State,
    /// Who started it.
    pub by: String,
    /// Why it paused or ended, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The changes merged under it.
    #[serde(default)]
    pub merged: Vec<u32>,
    /// Why it is held, while it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
    /// When each merge in the last [`SPAN`] was counted, for the rate breaker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<u64>,
    /// When the rate breaker held it, while that hold is the one it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet: Option<u64>,
}

impl Window {
    /// Whether it may approve now: active or held.
    #[must_use]
    pub fn usable(&self) -> bool {
        matches!(self.state, State::Active | State::Held)
    }

    /// Whether it is running: active, held or paused.
    fn running(&self) -> bool {
        matches!(self.state, State::Active | State::Held | State::Paused)
    }

    /// The merges counted in the [`SPAN`] before `now`.
    #[must_use]
    pub fn rate(&self, now: u64) -> usize {
        self.recent
            .iter()
            .filter(|t| now.saturating_sub(**t) < SPAN)
            .count()
    }
}
/// Every window: at most one active or paused, and the ended ones kept for the record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Windows {
    /// The windows, oldest first.
    #[serde(default)]
    pub list: Vec<Window>,
    /// The last number given.
    #[serde(default)]
    pub next: u32,
}

/// Whether `w` lets `operation` on `change` through at `now`: authorizing, approving, merging
/// and closing at the code tier or lower. The owner tier and break-glass still ask, and a hold
/// defers what it would allow.
#[must_use]
pub fn decide(w: &Window, change: &crate::change::Change, operation: &str, now: u64) -> Decision {
    match judge(w, change, operation, now) {
        Decision::Allow => waits(w, change, operation).map_or(Decision::Allow, Decision::Hold),
        d => d,
    }
}

fn judge(w: &Window, change: &crate::change::Change, operation: &str, _now: u64) -> Decision {
    if !w.usable() {
        return Decision::Ask(format!("Auto window {} has ended or paused", w.id));
    }
    if !matches!(
        operation,
        "change.authorize" | "change.approve" | "change.merge" | "change.close"
    ) {
        return Decision::Ask(format!("`{operation}` isn't something Auto approves"));
    }
    if matches!(operation, "change.approve" | "change.merge") && change.tiers.is_none() {
        return Decision::Ask(format!("change {}'s head has no tiers yet", change.number));
    }
    if matches!(operation, "change.approve" | "change.merge")
        && change.tiers.as_ref().is_some_and(|t| {
            (t.merge.rule.starts_with("guarded")
                || crate::approval::tiers::is_judge_rule(&t.merge.rule))
                && !t.merge.rule.ends_with(crate::approval::tiers::AUTO_COVERS)
        })
    {
        return Decision::Ask(format!(
            "change {} touches a gate path, or a guarded or eviction one without a gate set, which \
             Auto never covers",
            change.number
        ));
    }
    match tier_of(change, operation) {
        Tier::Owner => Decision::Ask(format!(
            "change {} needs the owner each time, Auto or not",
            change.number
        )),
        _ => Decision::Allow,
    }
}

impl Windows {
    /// The window that is active, held or paused, if any.
    #[must_use]
    pub fn current(&self) -> Option<&Window> {
        self.list.iter().rev().find(|w| w.running())
    }

    /// Pauses a window whose hold is spent; holds one whose merges trip the rate breaker, and
    /// releases that hold, with a fresh count, after [`QUIET`] without a counted merge.
    #[must_use]
    pub fn settle(&self, now: u64) -> Self {
        let mut next = self.clone();
        for w in &mut next.list {
            w.recent.retain(|t| now.saturating_sub(*t) < SPAN);
            let spent = w.hold.as_ref().and_then(|h| h.spent(now));
            if let (State::Held, Some(why)) = (w.state, spent) {
                (w.state, w.why, w.hold, w.quiet) = (State::Paused, Some(why), None, None);
            }
            if let (State::Held, Some(since)) = (w.state, w.quiet) {
                let last = w.recent.last().copied().unwrap_or(since).max(since);
                if now.saturating_sub(last) >= QUIET {
                    (w.state, w.why, w.hold, w.quiet) = (State::Active, None, None, None);
                    w.recent.clear();
                }
            } else if w.state == State::Active && w.rate(now) >= RATE {
                w.why = Some(rate_waits());
                (w.state, w.quiet) = (State::Held, Some(now));
            }
        }
        let ended = next.list.iter().filter(|w| w.state == State::Ended).count();
        if ended > KEEP {
            let mut drop = ended - KEEP;
            next.list.retain(|w| {
                let gone = drop > 0 && w.state == State::Ended;
                if gone {
                    drop -= 1;
                }
                !gone
            });
        }
        next
    }

    /// Starts a standing window for `by` at `now`, ending any held or paused one.
    ///
    /// # Errors
    ///
    /// When a window is already active.
    pub fn start(&self, by: &str, now: u64) -> Result<(Self, Window), String> {
        let mut next = self.settle(now);
        if next.list.iter().any(|w| w.state == State::Active) {
            return Err("an Auto window is already running".into());
        }
        for w in next.list.iter_mut().filter(|w| w.running()) {
            (w.state, w.hold) = (State::Ended, None);
            w.why = Some("a new window started".into());
        }
        next.next += 1;
        let w = Window {
            id: next.next,
            hours: 0,
            started: now,
            expires: now,
            used: 0,
            state: State::Active,
            by: by.to_string(),
            why: None,
            merged: Vec::new(),
            hold: None,
            recent: Vec::new(),
            quiet: None,
        };
        next.list.push(w.clone());
        Ok((next, w))
    }

    /// Ends the current window, saying `why`.
    ///
    /// # Errors
    ///
    /// When none is active or paused.
    pub fn end(&self, why: &str) -> Result<(Self, Window), String> {
        let mut next = self.clone();
        let Some(w) = next.list.iter_mut().rev().find(|w| w.running()) else {
            return Err("no Auto window is running".into());
        };
        (w.state, w.why, w.hold) = (State::Ended, Some(why.to_string()), None);
        let w = w.clone();
        Ok((next, w))
    }

    /// Pauses the active window, saying `why`, as when a deploy fails: the window paused.
    #[must_use]
    pub fn pause_all(&self, why: &str) -> (Self, Option<u32>) {
        let mut next = self.clone();
        let paused = next
            .list
            .iter_mut()
            .find(|w| w.state == State::Active)
            .map(|w| {
                w.state = State::Paused;
                w.why = Some(why.to_string());
                w.id
            });
        (next, paused)
    }

    /// Holds the active or held window after the deploy of `sha` failed for `why` at `now`,
    /// pausing it when its hold is spent: the next windows, and the window and whether it paused.
    #[must_use]
    pub fn hold_all(&self, sha: &str, why: &str, now: u64) -> (Self, Option<(u32, bool)>) {
        let mut next = self.settle(now);
        let Some(w) = next
            .list
            .iter_mut()
            .find(|w| matches!(w.state, State::Active | State::Held))
        else {
            return (next, None);
        };
        let h = Hold::after(w.hold.as_ref(), sha, why, now);
        w.quiet = None;
        let ended = h.spent(now);
        (w.state, w.why, w.hold) = match ended.clone() {
            Some(why) => (State::Paused, Some(why), None),
            None => (State::Held, Some(h.waits()), Some(h)),
        };
        let id = w.id;
        (next, Some((id, ended.is_some())))
    }

    /// Resumes the window a failed deploy held at `now`, not one the rate breaker holds: the next
    /// windows, and the window resumed.
    #[must_use]
    pub fn resume_all(&self, now: u64) -> (Self, Option<Window>) {
        let mut next = self.settle(now);
        let Some(w) = next
            .list
            .iter_mut()
            .find(|w| w.state == State::Held && w.quiet.is_none())
        else {
            return (next, None);
        };
        (w.state, w.why, w.hold) = (State::Active, None, None);
        let w = w.clone();
        (next, Some(w))
    }

    /// Decides `operation` on `change` under the active or held window, recording a merge: the next
    /// windows, and the window and decision, or none when no window is active.
    #[must_use]
    pub fn decide(
        &self,
        change: &crate::change::Change,
        operation: &str,
        now: u64,
    ) -> (Self, Option<(u32, Decision)>) {
        let mut next = self.settle(now);
        let Some(w) = next
            .list
            .iter_mut()
            .find(|w| matches!(w.state, State::Active | State::Held))
        else {
            return (next, None);
        };
        let d = decide(w, change, operation, now);
        if merges(w, change, operation, &d) {
            w.used += 1;
            w.merged.push(change.number);
            w.recent.push(now);
        }
        let id = w.id;
        (next.settle(now), Some((id, d)))
    }
}

/// Whether `d` on `operation` merges `change` under `w` for the first time: an allowed merge, or
/// an allowed approval, which queues its merge unless a hold defers it.
fn merges(w: &Window, change: &crate::change::Change, operation: &str, d: &Decision) -> bool {
    let queues = match operation {
        "change.merge" => true,
        "change.approve" => waits(w, change, "change.merge").is_none(),
        _ => false,
    };
    *d == Decision::Allow && queues && !w.merged.contains(&change.number)
}

/// A window's move between `before` and `after`: held, released, paused or ended without the
/// owner, for the event and push that tell it; see the standing Auto ADR's 2026-11-15 amendment
/// on the rate breaker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    /// The window.
    pub id: u32,
    /// Where it was.
    pub from: State,
    /// Where it is.
    pub to: State,
    /// Why, as the window says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// Each window whose state differs between `before` and `after`.
#[must_use]
pub fn moved(before: &Windows, after: &Windows) -> Vec<Moved> {
    after
        .list
        .iter()
        .filter_map(|w| {
            let was = before.list.iter().find(|b| b.id == w.id)?;
            (was.state != w.state).then(|| Moved {
                id: w.id,
                from: was.state,
                to: w.state,
                why: w.why.clone(),
            })
        })
        .collect()
}

/// What the rate breaker's hold says a merge waits on.
fn rate_waits() -> String {
    format!(
        "merges once an hour passes with none: {RATE} merges in {} hours; `fix` and `revert` go through",
        SPAN / 3_600
    )
}

/// What `w`'s hold, a failed deploy's or the rate breaker's, defers `operation` on `change` on:
/// only a merge that isn't a `fix` or `revert`.
fn waits(w: &Window, change: &crate::change::Change, operation: &str) -> Option<String> {
    defers(w.hold.as_ref(), change, operation).or_else(|| {
        (w.quiet.is_some() && operation == "change.merge" && !fixes(change)).then(rate_waits)
    })
}
