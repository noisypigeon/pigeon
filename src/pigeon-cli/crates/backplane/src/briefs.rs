//! The `Instance`'s store of briefs: numbered, append-only, and amended by their planner only;
//! see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::brief::{Brief, Draft};

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
}
