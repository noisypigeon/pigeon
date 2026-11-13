//! Several agent sessions at once: what a proposal overlaps among the open changes; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §1, §2.

use serde::{Deserialize, Serialize};

use crate::approvals;

/// The longest session label.
pub const MAX_SESSION: usize = 200;

/// A session label, trimmed: one line of at most [`MAX_SESSION`] characters, or none.
///
/// # Errors
///
/// When it is longer, or holds a control character.
pub fn session(label: Option<&str>) -> Result<Option<String>, String> {
    let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) else {
        return Ok(None);
    };
    if label.chars().count() > MAX_SESSION || label.chars().any(char::is_control) {
        return Err(format!(
            "`session` is one line of at most {MAX_SESSION} characters"
        ));
    }
    Ok(Some(label.to_string()))
}

/// An open change, as D1 mirrors it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Open {
    /// Its number.
    pub number: u32,
    /// Its state.
    pub state: String,
    /// Its session, if it named one.
    #[serde(default)]
    pub session: Option<String>,
    /// Its scope.
    pub scope: Vec<String>,
    /// What its ADR will govern.
    #[serde(default)]
    pub affects: Vec<String>,
    /// Its plan.
    #[serde(default)]
    pub plan: String,
    /// Its slug.
    #[serde(default)]
    pub slug: String,
}

/// An open change a proposal overlaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlap {
    /// Its number.
    pub number: u32,
    /// Its state.
    pub state: String,
    /// Its session, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Its globs, from its scope or `affects`, that meet the proposal's.
    pub globs: Vec<String>,
    /// Paths both name exactly: both may create the same file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub same: Vec<String>,
}

/// Whether globs `a` and `b` may reach a common path.
#[must_use]
pub fn meet(a: &str, b: &str) -> bool {
    approvals::overlaps(a, b) || approvals::overlaps(b, a)
}

/// The open changes whose scope or `affects` meets `scope` or `affects`, lowest number first.
#[must_use]
pub fn overlaps(scope: &[String], affects: &[String], open: &[Open]) -> Vec<Overlap> {
    let mine: Vec<&String> = scope.iter().chain(affects).collect();
    let mut out: Vec<Overlap> = open
        .iter()
        .filter_map(|o| {
            let theirs = o.scope.iter().chain(&o.affects);
            let mut globs: Vec<String> = theirs
                .clone()
                .filter(|t| mine.iter().any(|m| meet(m, t)))
                .cloned()
                .collect();
            globs.dedup();
            if globs.is_empty() {
                return None;
            }
            let same = theirs
                .filter(|t| !t.contains(['*', '?']) && mine.iter().any(|m| m == t))
                .cloned()
                .collect();
            Some(Overlap {
                number: o.number,
                state: o.state.clone(),
                session: o.session.clone(),
                globs,
                same,
            })
        })
        .collect();
    out.sort_by_key(|o| o.number);
    out
}

/// The most open changes one session holds; see the concurrent-sessions ADR's 2026-11-10
/// amendment.
pub const MAX_PER_SESSION: usize = 5;

/// The most open changes an instance holds.
pub const MAX_OPEN: usize = 20;

/// The change a generated change was made for, from its slug: `reconcile-<n>` and
/// `review-<n>-<m>` belong to `n`; none for any other change.
#[must_use]
pub fn family(slug: &str) -> Option<u32> {
    let rest = slug
        .strip_prefix("reconcile-")
        .or_else(|| slug.strip_prefix("review-"))?;
    rest.split('-').next()?.parse().ok()
}

/// Whether `o` takes a slot: every change but a generated one that never pushed, which holds
/// its parent's; see `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §2.
fn counts(o: &Open) -> bool {
    family(&o.slug).is_none() || !matches!(o.state.as_str(), "proposed" | "planned")
}

/// Whether a proposal for `session` fits beside the `open` changes. A change and its unpushed
/// generated changes hold one slot.
///
/// # Errors
///
/// Which limit it passes, naming the changes that count, each with its unpushed family.
pub fn limit(open: &[Open], session: Option<&str>) -> Result<(), String> {
    let numbers = |o: &[&Open]| {
        o.iter()
            .map(|o| {
                let kin: Vec<String> = open
                    .iter()
                    .filter(|g| !counts(g) && family(&g.slug) == Some(o.number))
                    .map(|g| format!("+!{}", g.number))
                    .collect();
                if kin.is_empty() {
                    format!("!{}", o.number)
                } else {
                    format!("!{} ({})", o.number, kin.join(" "))
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let counted: Vec<&Open> = open.iter().filter(|o| counts(o)).collect();
    if counted.len() >= MAX_OPEN {
        return Err(format!(
            "{MAX_OPEN} changes are open ({}); merge or close one first",
            numbers(&counted)
        ));
    }
    if let Some(s) = session {
        let mine: Vec<&Open> = counted
            .iter()
            .copied()
            .filter(|o| o.session.as_deref() == Some(s))
            .collect();
        if mine.len() >= MAX_PER_SESSION {
            return Err(format!(
                "session {s} holds {MAX_PER_SESSION} open changes ({}); merge or close one first",
                numbers(&mine)
            ));
        }
    }
    Ok(())
}
