//! What briefs cost, by class and ADR step, for the digest; see
//! `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §7.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Brief;
use crate::sessions::Class;

/// How long a step stays in the digest after its newest brief revision, in seconds.
pub const RECENT: u64 = 7 * 24 * 60 * 60;

/// A brief's running counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    /// Heads its changes reached.
    #[serde(default)]
    pub heads: u32,
    /// Its planner's verdicts.
    #[serde(default)]
    pub reviews: u32,
    /// Of those, `revise`.
    #[serde(default)]
    pub revised: u32,
    /// Its executors' escalations.
    #[serde(default)]
    pub escalations: u32,
}

impl Tally {
    /// Whether nothing is counted.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    /// `self` plus `other`.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self {
            heads: self.heads.saturating_add(other.heads),
            reviews: self.reviews.saturating_add(other.reviews),
            revised: self.revised.saturating_add(other.revised),
            escalations: self.escalations.saturating_add(other.escalations),
        }
    }
}

/// An ADR's file name without its directory, date or extension.
fn adr_name(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let file = file.strip_suffix(".md").unwrap_or(file);
    match file.get(..11) {
        Some(d) if d.len() == 11 && d.chars().take(10).all(|c| c.is_ascii_digit() || c == '-') => {
            &file[11..]
        }
        _ => file,
    }
}

/// Whether `b` is still being carried out: no change yet, or its latest change open in `open`.
fn active(b: &Brief, open: &BTreeMap<u32, String>) -> bool {
    b.changes.last().is_none_or(|n| open.contains_key(n))
}

/// The digest's brief lines at `now`: one per active brief, with its change's stage from `open`
/// (each open change's stage), then one per ADR step with an active brief or a revision within
/// [`RECENT`], its briefs counted by class.
#[must_use]
pub fn lines(briefs: &[Brief], open: &BTreeMap<u32, String>, now: u64) -> Vec<String> {
    let mut out = Vec::new();
    let mut steps: BTreeMap<(String, String), BTreeMap<Class, (u32, Tally)>> = BTreeMap::new();
    for b in briefs {
        let r = b.latest();
        let d = &r.draft;
        let live = active(b, open);
        if live {
            let at = b.changes.last().map_or_else(
                || "unclaimed".to_string(),
                |n| format!("!{n} {}", open.get(n).map_or("", String::as_str)),
            );
            out.push(format!(
                "brief {} ({}, {} step {}): {at}",
                b.id,
                d.model.name(),
                adr_name(&d.adr),
                d.step
            ));
        }
        if live || now.saturating_sub(r.at) <= RECENT {
            let by = steps
                .entry((adr_name(&d.adr).to_string(), d.step.clone()))
                .or_default()
                .entry(d.model)
                .or_default();
            *by = (by.0 + 1, by.1.plus(b.tally));
        }
    }
    for ((adr, step), by) in steps {
        let classes: Vec<String> = by
            .iter()
            .map(|(class, (n, t))| {
                format!(
                    "{} {n} brief(s), {} head(s), {} review(s) ({} revised), {} escalation(s)",
                    class.name(),
                    t.heads,
                    t.reviews,
                    t.revised,
                    t.escalations
                )
            })
            .collect();
        out.push(format!("step {adr} {step}: {}", classes.join("; ")));
    }
    out
}
