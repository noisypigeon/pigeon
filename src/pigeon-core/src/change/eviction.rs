//! An open change as an eviction candidate: the fields sentinel's `eviction` target judges; see
//! `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What the control plane knows of one open change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    /// Its number.
    pub number: u32,
    /// Its slug, which names a generated change's kind.
    pub slug: String,
    /// Its state.
    pub state: String,
    /// Its head, once pushed.
    pub head: Option<String>,
    /// When its record last changed, in Unix seconds.
    pub updated: u64,
    /// Its session, if it named one.
    pub session: Option<Session>,
    /// What holds it from eviction.
    pub holds: Holds,
}

/// A change's session, as its ticket stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Its label.
    pub label: String,
    /// Whether it holds a live ticket.
    pub live: bool,
    /// How long it has had no live ticket, when known.
    pub gone_for: Option<u64>,
}

/// What keeps a change from eviction whatever its session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holds {
    /// The owner pinned it.
    pub pinned: bool,
    /// An owner approval page waits on it.
    pub approval_open: bool,
    /// It waits in, or holds, the merge queue.
    pub queued: bool,
}

/// `slug`'s kind: `reconcile`, `review`, or `change`.
#[must_use]
pub fn kind(slug: &str) -> &'static str {
    match slug.split('-').next() {
        Some("reconcile") if slug.len() > "reconcile-".len() => "reconcile",
        Some("review") if slug.len() > "review-".len() => "review",
        _ => "change",
    }
}

/// `f` as the fields an `eviction` policy reads, at `now`. A field that isn't known is left
/// out, so no policy that names it grants.
#[must_use]
pub fn candidate(f: &Facts, now: u64) -> BTreeMap<String, String> {
    let live = f.session.as_ref().is_some_and(|s| s.live);
    let mut c: BTreeMap<String, String> = [
        ("number", f.number.to_string()),
        ("kind", kind(&f.slug).to_string()),
        ("state", f.state.clone()),
        ("pushed", f.head.is_some().to_string()),
        ("session_live", live.to_string()),
        ("idle", now.saturating_sub(f.updated).to_string()),
        ("pinned", f.holds.pinned.to_string()),
        ("approval_open", f.holds.approval_open.to_string()),
        ("queued", f.holds.queued.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    // A change proposed without a session is gone once idle as long as a session would be.
    let gone = match &f.session {
        None => Some(now.saturating_sub(f.updated)),
        Some(s) if s.live => Some(0),
        Some(s) => s.gone_for,
    };
    if let Some(g) = gone {
        c.insert("session_gone_for".into(), g.to_string());
    }
    c
}

/// The most changes evicted in one hourly run; see the eviction ADR §5.
pub const PER_RUN: usize = 6;

/// How old a verdict may be and still be acted on, in seconds.
pub const FRESH: u64 = 2 * 60 * 60;

/// What an evicted change had done, what was left, and how to resume it; see the eviction ADR §6.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remains {
    /// What was done.
    #[serde(default)]
    pub done: Vec<String>,
    /// What was left.
    #[serde(default)]
    pub left: Vec<String>,
    /// How a new session resumes it.
    #[serde(default)]
    pub resume: String,
}

/// An eviction: the policy that granted it, when, the head kept, and what remains.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evicted {
    /// The policy that granted it, `policy:<name>`.
    pub policy: String,
    /// When, in Unix seconds.
    pub at: u64,
    /// The head kept on its branch, if it pushed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// What remains.
    #[serde(default)]
    pub remains: Remains,
}

/// Whether `c` was more than a plan: authorized, or pushed.
#[must_use]
pub fn incomplete(c: &super::Change) -> bool {
    c.plan_approval.is_some() || c.head.is_some()
}

/// What `c` had done and has left, and how to resume it.
#[must_use]
pub fn remains(c: &super::Change) -> Remains {
    use super::{JobStatus, State};
    let mut done = vec![format!(
        "plan: {}",
        c.plan.lines().next().unwrap_or_default()
    )];
    if let Some(h) = &c.head {
        done.push(format!("head: {h}, kept on {}", c.branch()));
    }
    for j in &c.jobs {
        done.push(format!("{}: {:?}", j.name, j.status).to_lowercase());
    }
    done.extend(c.violations.iter().map(|v| format!("violation: {v}")));
    let failing: Vec<String> = c
        .jobs
        .iter()
        .filter(|j| j.status == JobStatus::Failed)
        .map(|j| j.name.clone())
        .collect();
    let left = if c.head.is_none() {
        vec!["the whole plan: nothing was pushed".to_string()]
    } else if let Some(conflict) = &c.conflict {
        vec![format!(
            "rebase onto main; these paths conflict: {}",
            conflict.paths.join(", ")
        )]
    } else if !failing.is_empty() {
        vec![format!("fix the failing jobs: {}", failing.join(", "))]
    } else if c.behind.is_some() {
        vec!["rebase onto main and push".to_string()]
    } else if c.state == State::Checked {
        vec!["only the owner's approval".to_string()]
    } else {
        vec![format!("its checks, from state `{}`", c.state.name())]
    };
    let resume = format!(
        "change_propose with `resumes: {}`, then fetch {} and push its head to the new branch",
        c.number,
        c.branch()
    );
    Remains { done, left, resume }
}

/// The candidates to evict from `verdicts`, judged at `judged_at`: each one a policy granted,
/// lowest number first, at most [`PER_RUN`]; none when the verdicts are older than [`FRESH`].
#[must_use]
pub fn pick(verdicts: &[serde_json::Value], judged_at: u64, now: u64) -> Vec<(u32, String)> {
    if now.saturating_sub(judged_at) > FRESH {
        return Vec::new();
    }
    let mut granted: Vec<(u32, String)> = verdicts
        .iter()
        .filter_map(|v| {
            let n = u32::try_from(v["number"].as_u64()?).ok()?;
            Some((n, v["granted"].as_str()?.to_string()))
        })
        .collect();
    granted.sort();
    granted.truncate(PER_RUN);
    granted
}
