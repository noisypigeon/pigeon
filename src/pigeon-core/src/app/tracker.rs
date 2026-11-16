//! The app's tracker: each session and its changes on a ten-stop rail; see .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendment.

use serde::{Deserialize, Serialize};

use crate::brief::Brief;
use crate::change::stage::{Needs, Stage};

/// The rail's stops, in order.
pub const STOPS: [&str; 10] = [
    "proposed",
    "authorized",
    "pushed",
    "checking",
    "checked",
    "approved",
    "queued",
    "merging",
    "merged",
    "deploying",
];

/// How a change stands at its stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// Moving.
    Ok,
    /// Refused or failed.
    Failed,
    /// Needs a session that isn't listening.
    Stalled,
    /// Finished.
    Done,
    /// Held for later; see the hold-and-resume amendment (!484).
    Held,
}

/// The stop and mark of a change at `stage`, when `stalled`; none for `closed` or an unknown
/// stage.
#[must_use]
pub fn rail(stage: &str, stalled: bool) -> Option<(u8, Mark)> {
    let (stop, base) = match stage {
        "proposed" => (0, Mark::Ok),
        "authorized" => (1, Mark::Ok),
        "bounding" | "conflicted" | "restacking" => (2, Mark::Ok),
        "refused" => (2, Mark::Failed),
        "checking" => (3, Mark::Ok),
        "failed" => (3, Mark::Failed),
        "checked" | "in-review" | "escalated" => (4, Mark::Ok),
        "approved" => (5, Mark::Ok),
        "held" => return Some((5, Mark::Held)),
        "merging" => (7, Mark::Ok),
        "merged" => (8, Mark::Ok),
        "deploying" => (9, Mark::Ok),
        "done" => (9, Mark::Done),
        "deploy-failed" => (9, Mark::Failed),
        s if s.starts_with("checking ") => (3, Mark::Ok),
        s if s.starts_with("queued ") => (6, Mark::Ok),
        _ => return None,
    };
    let mark = if stalled && base == Mark::Ok {
        Mark::Stalled
    } else {
        base
    };
    Some((stop, mark))
}

/// The brief a change carries out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefRef {
    /// `b<n>`.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its ADR.
    pub adr: String,
    /// Its ADR step.
    pub step: String,
}

/// A change on the rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// Its number.
    pub number: u32,
    /// Its title.
    pub title: String,
    /// Its stage.
    pub stage: String,
    /// Who acts next.
    pub needs: Needs,
    /// For `Needs::Change`: the change it waits on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<u32>,
    /// What acting next is.
    pub next: String,
    /// When it entered its stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// Its stop on the rail.
    pub stop: u8,
    /// How it stands.
    pub mark: Mark,
    /// The brief it carries out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<BriefRef>,
}

/// A session and its changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Its label.
    pub label: String,
    /// Its class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::sessions::Class>,
    /// Its role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Its planner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<String>,
    /// The brief its ticket declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Whether it is waiting on events now.
    pub listening: bool,
    /// Its changes, by number.
    pub changes: Vec<Row>,
}

/// How many changes wait on whom.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Changes that need the owner.
    pub owner: u32,
    /// Changes marked stalled.
    pub stalled: u32,
    /// Changes that need a provider.
    pub provider: u32,
}

/// A change in the merge queue; see `.claude/decisions/2026-11-15-runtime-ios-app-now-tab.md` §4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    /// Its number.
    pub number: u32,
    /// Its title.
    pub title: String,
    /// Its session's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Its session's class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::sessions::Class>,
    /// Its stage.
    pub stage: String,
    /// 0 while it merges, else its place in the queue.
    pub position: u32,
    /// Who acts next.
    pub needs: Needs,
    /// For `Needs::Change`: the change it waits on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<u32>,
    /// What acting next is.
    pub next: String,
    /// When it entered its stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// The tracker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracker {
    /// Its sessions.
    pub sessions: Vec<Session>,
    /// Its counts.
    pub counts: Counts,
    /// The merge queue: the change merging, then each queued, in order.
    #[serde(default)]
    pub queue: Vec<Queued>,
}

/// A session's inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIn {
    /// Its label.
    pub label: String,
    /// What its ticket declares; none without a live ticket.
    pub declared: Option<crate::sessions::Declared>,
    /// The class its latest ticket declared, live or lapsed.
    pub class: Option<crate::sessions::Class>,
    /// When it last polled.
    pub polled: Option<u64>,
    /// Whether it is waiting on events now.
    pub listening: bool,
}

/// A change's inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeIn {
    /// Its number.
    pub number: u32,
    /// Its title.
    pub title: String,
    /// Its session's label.
    pub session: Option<String>,
    /// Its stage.
    pub stage: Stage,
}

const fn needs_name(n: Needs) -> &'static str {
    match n {
        Needs::Agent => "agent",
        Needs::Owner => "owner",
        Needs::Platform => "platform",
        Needs::Provider => "provider",
        Needs::Change => "change",
        Needs::None => "none",
    }
}

fn brief_ref(b: &Brief) -> Option<BriefRef> {
    let d = &b.revisions.last()?.draft;
    Some(BriefRef {
        id: b.id.clone(),
        title: d.title.clone(),
        adr: d.adr.clone(),
        step: d.step.clone(),
    })
}

/// The tracker at `now`: `changes` on the rail under `sessions` and any session they name.
#[must_use]
pub fn build(sessions: &[SessionIn], changes: &[ChangeIn], briefs: &[Brief], now: u64) -> Tracker {
    let mut labels: Vec<&str> = sessions.iter().map(|s| s.label.as_str()).collect();
    for c in changes {
        if let Some(l) = c.session.as_deref() {
            labels.push(l);
        }
    }
    labels.sort_unstable();
    labels.dedup();

    let mut counts = Counts::default();
    let mut out = Vec::new();
    for label in labels {
        let known = sessions.iter().find(|s| s.label == label);
        let declared = known.and_then(|s| s.declared.as_ref());
        let class = declared
            .map(|d| d.model)
            .or_else(|| known.and_then(|s| s.class));
        let polled = known.and_then(|s| s.polled);
        let mut rows = Vec::new();
        for c in changes
            .iter()
            .filter(|c| c.session.as_deref() == Some(label))
        {
            let needs = needs_name(c.stage.needs);
            let stalled = crate::tasks::stalled(polled, c.stage.since, needs, now).is_some();
            let Some((stop, mark)) = rail(&c.stage.stage, stalled) else {
                continue;
            };
            let brief = briefs
                .iter()
                .find(|b| b.changes.contains(&c.number))
                .or_else(|| {
                    declared
                        .and_then(|d| d.brief.as_deref())
                        .and_then(|id| briefs.iter().find(|b| b.id == id))
                })
                .and_then(brief_ref);
            match c.stage.needs {
                Needs::Owner => counts.owner += 1,
                Needs::Provider => counts.provider += 1,
                _ => {}
            }
            if mark == Mark::Stalled {
                counts.stalled += 1;
            }
            rows.push(Row {
                number: c.number,
                title: c.title.clone(),
                stage: c.stage.stage.clone(),
                needs: c.stage.needs,
                waits_on: c.stage.waits_on,
                next: c.stage.next.clone(),
                since: c.stage.since,
                stop,
                mark,
                brief,
            });
        }
        rows.sort_by_key(|r| r.number);
        out.push(Session {
            label: label.to_string(),
            model: class,
            role: class.map(|c| c.role().to_string()),
            planner: declared.and_then(|d| d.planner.clone()),
            brief: declared.and_then(|d| d.brief.clone()),
            listening: known.is_some_and(|s| s.listening),
            changes: rows,
        });
    }
    out.sort_by_key(|s| {
        (
            !s.changes.iter().any(|r| r.needs == Needs::Owner),
            !s.changes.iter().any(|r| r.mark == Mark::Stalled),
            s.label.clone(),
        )
    });
    let queue = queue(changes, &out);
    Tracker {
        sessions: out,
        counts,
        queue,
    }
}

/// A change's place in the merge queue: 0 while `merging`, `n` at `queued n/m`; none otherwise.
#[must_use]
pub fn position(stage: &str) -> Option<u32> {
    if stage == "merging" {
        return Some(0);
    }
    let rest = stage.strip_prefix("queued ")?;
    rest.split_once('/').map_or(rest, |(n, _)| n).parse().ok()
}

/// The merge queue in order, each change with its session's class from `sessions`.
fn queue(changes: &[ChangeIn], sessions: &[Session]) -> Vec<Queued> {
    let mut out: Vec<Queued> = changes
        .iter()
        .filter_map(|c| {
            let position = position(&c.stage.stage)?;
            let model = c
                .session
                .as_deref()
                .and_then(|l| sessions.iter().find(|s| s.label == l))
                .and_then(|s| s.model);
            Some(Queued {
                number: c.number,
                title: c.title.clone(),
                session: c.session.clone(),
                model,
                stage: c.stage.stage.clone(),
                position,
                needs: c.stage.needs,
                waits_on: c.stage.waits_on,
                next: c.stage.next.clone(),
                since: c.stage.since,
            })
        })
        .collect();
    out.sort_by_key(|q| (q.position, q.number));
    out
}
