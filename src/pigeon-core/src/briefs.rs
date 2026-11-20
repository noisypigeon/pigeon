//! The `Instance`'s store of briefs: numbered, append-only, and amended by their planner only;
//! see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::brief::report::Tally;
use crate::brief::{Brief, Draft};

/// Every brief of an instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Briefs {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// Each brief, by number.
    #[serde(default)]
    pub briefs: BTreeMap<u32, Brief>,
}

/// A brief's number from `b<n>` or `<n>`.
#[must_use]
pub fn number(id: &str) -> Option<u32> {
    let id = id.trim();
    id.strip_prefix('b')
        .unwrap_or(id)
        .parse()
        .ok()
        .filter(|n| *n > 0)
}

impl Briefs {
    /// Stores `draft` as a new brief by `planner` at `now`.
    ///
    /// # Errors
    ///
    /// Every problem with the draft.
    pub fn create(&mut self, planner: &str, draft: Draft, now: u64) -> Result<Brief, Vec<String>> {
        let brief = Brief::create(self.last + 1, planner, draft, now)?;
        self.last += 1;
        self.briefs.insert(self.last, brief.clone());
        Ok(brief)
    }

    /// Appends `draft` to brief `id` as its next revision, by `by`, for `reason`.
    ///
    /// # Errors
    ///
    /// When there's no such brief, or the amendment is refused.
    pub fn amend(
        &mut self,
        id: &str,
        by: &str,
        draft: Draft,
        reason: &str,
        now: u64,
    ) -> Result<Brief, Vec<String>> {
        let n = number(id).ok_or_else(|| vec![format!("no brief `{id}`")])?;
        let current = self
            .briefs
            .get(&n)
            .ok_or_else(|| vec![format!("no brief b{n}")])?;
        let next = current.amend(by, draft, reason, now)?;
        self.briefs.insert(n, next.clone());
        Ok(next)
    }

    /// Records change `change` as carrying out brief `id`.
    ///
    /// # Errors
    ///
    /// When there's no such brief.
    pub fn bind(&mut self, id: &str, change: u32) -> Result<&Brief, String> {
        let b = number(id)
            .and_then(|n| self.briefs.get_mut(&n))
            .ok_or_else(|| format!("no brief `{id}`"))?;
        if !b.changes.contains(&change) {
            b.changes.push(change);
        }
        Ok(b)
    }

    /// Adds `by` to brief `id`'s tally.
    ///
    /// # Errors
    ///
    /// When there's no such brief.
    pub fn tally(&mut self, id: &str, by: Tally) -> Result<&Brief, String> {
        let b = number(id)
            .and_then(|n| self.briefs.get_mut(&n))
            .ok_or_else(|| format!("no brief `{id}`"))?;
        b.tally = b.tally.plus(by);
        Ok(b)
    }

    /// Brief `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Brief> {
        number(id).and_then(|n| self.briefs.get(&n))
    }

    /// The briefs, newest first, by `planner` when named.
    #[must_use]
    pub fn list(&self, planner: Option<&str>) -> Vec<&Brief> {
        self.briefs
            .values()
            .rev()
            .filter(|b| planner.is_none_or(|p| b.planner == p))
            .collect()
    }

    /// The briefs finished at `now`: each with a change whose latest isn't in `open`, and whose
    /// newest revision is older than [`crate::brief::report::RECENT`], so the digest no longer
    /// counts it. See the briefs ADR's 2026-11-20 archive amendment.
    #[must_use]
    pub fn finished(&self, open: &std::collections::BTreeSet<u32>, now: u64) -> Vec<u32> {
        self.briefs
            .iter()
            .filter(|(_, b)| {
                b.changes.last().is_some_and(|c| !open.contains(c))
                    && now.saturating_sub(b.latest().at) > crate::brief::report::RECENT
            })
            .map(|(n, _)| *n)
            .collect()
    }

    /// Takes brief `n` out of the store, as the archive takes it.
    pub fn take(&mut self, n: u32) -> Option<Brief> {
        self.briefs.remove(&n)
    }

    /// Puts an archived `brief` back under its number, before a write to it; `last` never falls.
    pub fn put_back(&mut self, brief: Brief) {
        if let Some(n) = number(&brief.id) {
            self.last = self.last.max(n);
            self.briefs.insert(n, brief);
        }
    }
}
