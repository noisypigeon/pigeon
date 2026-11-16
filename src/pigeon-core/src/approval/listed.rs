//! What a change owes the owner, and whether its listed approval is kept, issued or removed; see
//! `.claude/decisions/2026-11-15-security-owner-approvals-always-listed.md` §1-§4.

use crate::change::stage::{Needs, Stage};
use crate::change::{Change, State};

/// Re-issue a listed grant with this many seconds or fewer left.
pub const REISSUE_BEFORE: u64 = 2 * 60 * 60;

/// The approval a change owes the owner now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owed {
    /// The change.
    pub number: u32,
    /// The operation the owner runs.
    pub operation: &'static str,
    /// The head it names; none for a plan not yet pushed.
    pub head: Option<String>,
}

/// The change's entry in the inbox, as its grant names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The operation.
    pub operation: String,
    /// The head.
    pub head: Option<String>,
    /// When it expires, in Unix seconds.
    pub expires: u64,
}

/// What to do with the change's entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Leave it.
    Keep,
    /// Issue a fresh grant, and push when `push`.
    Issue {
        /// Whether to notify the owner.
        push: bool,
    },
    /// Take it from the inbox.
    Remove,
}

/// The approval the change owes the owner in `stage`, if any.
#[must_use]
pub fn owed(change: &Change, stage: &Stage) -> Option<Owed> {
    if stage.needs != Needs::Owner {
        return None;
    }
    let operation = match change.state {
        State::Proposed => "change.authorize",
        State::Checked if change.head.is_some() => "change.approve",
        _ => return None,
    };
    Some(Owed {
        number: change.number,
        operation,
        head: change.head.clone(),
    })
}

/// Whether a listed entry's `grant` still applies to `change`: known, unused, for its head, and
/// for an operation still offered; one that does stays listed, whoever asked for it.
#[must_use]
pub fn applies(change: &Change, grant: Option<&super::grants::Grant>) -> bool {
    grant.is_some_and(|g| g.used.is_none() && super::grants::still_applies(change, g))
}

/// What to do with the change's listed entry, given what it owes, at `now` in Unix seconds.
#[must_use]
pub fn reconcile(owed: Option<&Owed>, listed: Option<&Listed>, now: u64) -> Action {
    match (owed, listed) {
        (None, None) => Action::Keep,
        (None, Some(_)) => Action::Remove,
        (Some(_), None) => Action::Issue { push: true },
        (Some(o), Some(l)) => {
            if l.operation != o.operation || l.head != o.head {
                Action::Issue { push: true }
            } else if l.expires <= now + REISSUE_BEFORE {
                Action::Issue { push: false }
            } else {
                Action::Keep
            }
        }
    }
}

#[cfg(test)]
mod tests;
