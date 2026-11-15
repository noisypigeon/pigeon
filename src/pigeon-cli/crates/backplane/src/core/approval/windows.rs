//! Auto windows: the owner lets every change at the code tier or lower through for a few hours,
//! started and ended with a passkey; see
//! `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §5.

use serde::{Deserialize, Serialize};

use super::mandates::{Decision, Hold, State, defers};
use super::tiers::{Tier, tier_of};

/// The lengths a window may have, in hours.
pub const HOURS: [u32; 4] = [1, 4, 8, 24];
/// The merges a window allows.
pub const BUDGET: u32 = 20;
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
    /// How long it lasts, in hours.
    pub hours: u32,
    /// When it started, in Unix seconds.
    pub started: u64,
    /// When it ends, in Unix seconds.
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
}

impl Window {
    /// Whether it may approve now: active and unexpired, or held, and within its budget.
    #[must_use]
    pub fn usable(&self, now: u64) -> bool {
        let live = match self.state {
            State::Active => now < self.expires,
            State::Held => true,
            _ => false,
        };
        live && self.used < BUDGET
    }

    /// Whether it is running: active, held or paused.
    fn running(&self) -> bool {
        matches!(self.state, State::Active | State::Held | State::Paused)
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
pub fn decide(w: &Window, change: &crate::changes::Change, operation: &str, now: u64) -> Decision {
    match judge(w, change, operation, now) {
        Decision::Allow => {
            defers(w.hold.as_ref(), change, operation).map_or(Decision::Allow, Decision::Hold)
        }
        d => d,
    }
}

fn judge(w: &Window, change: &crate::changes::Change, operation: &str, now: u64) -> Decision {
    if !w.usable(now) {
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

    /// Ends a window whose time, budget or hold has run out.
    #[must_use]
    pub fn settle(&self, now: u64) -> Self {
        let mut next = self.clone();
        for w in &mut next.list {
            let spent = w.hold.as_ref().and_then(|h| h.spent(now));
            if let (State::Held, Some(end)) = (w.state, spent) {
                (w.state, w.why, w.hold) = (State::Ended, Some(end), None);
            }
            if w.state == State::Active && (now >= w.expires || w.used >= BUDGET) {
                w.state = State::Ended;
                w.why = Some(if w.used >= BUDGET {
                    "its 20 merges are used".into()
                } else {
                    "it ran its time".into()
                });
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

    /// Starts a window of `hours` for `by` at `now`, ending any paused one.
    ///
    /// # Errors
    ///
    /// When `hours` isn't 1, 4, 8 or 24, or a window is already active.
    pub fn start(&self, hours: u32, by: &str, now: u64) -> Result<(Self, Window), String> {
        if !HOURS.contains(&hours) {
            return Err("Auto lasts 1, 4, 8 or 24 hours".into());
        }
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
            hours,
            started: now,
            expires: now + u64::from(hours) * 3_600,
            used: 0,
            state: State::Active,
            by: by.to_string(),
            why: None,
            merged: Vec::new(),
            hold: None,
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
    /// ending it when its hold is spent: the next windows, and the window and whether it ended.
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
        let ended = h.spent(now);
        (w.state, w.why, w.hold) = match ended.clone() {
            Some(end) => (State::Ended, Some(end), None),
            None => (State::Held, Some(h.waits()), Some(h)),
        };
        let id = w.id;
        (next, Some((id, ended.is_some())))
    }

    /// Resumes the held window at `now`, its expiry moved later by the time held: the next
    /// windows, and the window resumed.
    #[must_use]
    pub fn resume_all(&self, now: u64) -> (Self, Option<Window>) {
        let mut next = self.settle(now);
        let Some(w) = next.list.iter_mut().find(|w| w.state == State::Held) else {
            return (next, None);
        };
        if let Some(h) = w.hold.take() {
            w.expires += now.saturating_sub(h.since);
        }
        (w.state, w.why) = (State::Active, None);
        let w = w.clone();
        (next, Some(w))
    }

    /// Decides `operation` on `change` under the active or held window, recording a merge: the next
    /// windows, and the window and decision, or none when no window is active.
    #[must_use]
    pub fn decide(
        &self,
        change: &crate::changes::Change,
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
        if d == Decision::Allow && operation == "change.merge" {
            w.used += 1;
            w.merged.push(change.number);
        }
        let id = w.id;
        (next.settle(now), Some((id, d)))
    }
}
