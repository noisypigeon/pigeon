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
