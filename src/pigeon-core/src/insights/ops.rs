//! The app's operational insights: deploys, merge waits, approval waits, throughput, providers and failed jobs; see .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md's 2026-11-15 amendment.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::DAY;
use crate::control::Statement;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

/// The ranges, in days, the app may ask for.
pub const RANGES: [u64; 3] = [1, 7, 30];

/// The answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ops {
    /// Days covered.
    pub range: u64,
    /// Deploys.
    pub deploys: Deploys,
    /// The merge queue.
    pub queue: Queue,
    /// Approval waits.
    pub approvals: Approvals,
    /// Merges.
    pub throughput: Throughput,
    /// Provider health.
    pub providers: Vec<Value>,
    /// Failed jobs.
    pub jobs: Vec<JobRow>,
    /// A caveat, if any.
    pub note: Option<String>,
}

/// Deploys in range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deploys {
    /// Those that passed.
    pub passed: u32,
    /// All.
    pub total: u32,
    /// The newest five.
    pub recent: Vec<DeployRow>,
}

/// One deploy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployRow {
    /// Its commit.
    pub sha: String,
    /// How it ended.
    pub outcome: String,
    /// When, in Unix seconds.
    pub at: u64,
}

/// The merge queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queue {
    /// Merges waiting.
    pub depth: u32,
    /// The change merging, if any.
    pub merging: Option<u32>,
    /// Minutes each merge waited after its approval.
    pub waits: Vec<u32>,
    /// Their median.
    pub p50: Option<u32>,
    /// Their longest.
    pub max: Option<u32>,
}

/// Waits for the owner's approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approvals {
    /// How many.
    pub count: u32,
    /// Median minutes.
    pub p50: Option<u32>,
    /// 95th percentile minutes.
    pub p95: Option<u32>,
}

/// Merges per day and lead time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Throughput {
    /// Each UTC day.
    pub days: Vec<Day>,
    /// Median minutes from proposal to merge.
    pub lead_p50: Option<u32>,
    /// Merges by who approved.
    pub by: By,
}

/// One UTC day's merges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// Its midnight, in Unix seconds.
    pub day: u64,
    /// Merges in it.
    pub merged: u32,
}

/// Merges by approver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct By {
    /// The owner.
    pub owner: u32,
    /// An Auto window.
    pub window: u32,
    /// A mandate.
    pub mandate: u32,
    /// A policy.
    pub policy: u32,
}

/// A failed job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRow {
    /// Its name.
    pub name: String,
    /// Its change.
    pub change: Option<u32>,
    /// How it ended.
    pub status: String,
    /// When, in Unix seconds.
    pub at: u64,
}

/// The changes merged since `from`.
#[must_use]
pub fn merged_since(instance: &str, repo: &str, from: u64) -> Statement {
    Statement {
        sql: "SELECT number, created, updated, approver FROM changes WHERE instance = ?1 \
              AND repo = ?2 AND state = 'merged' AND updated >= ?3",
        params: vec![json!(instance), json!(repo), json!(from)],
    }
}

/// The code-check jobs that failed or were cancelled since `from`.
#[must_use]
pub fn failed_jobs_since(instance: &str, repo: &str, from: u64) -> Statement {
    Statement {
        sql: "SELECT name, change, status, updated FROM jobs WHERE instance = ?1 AND repo = ?2 \
              AND status IN ('failed', 'cancelled') AND updated >= ?3 ORDER BY updated DESC \
              LIMIT 10",
        params: vec![json!(instance), json!(repo), json!(from)],
    }
}

/// The macOS jobs lost, unassigned or refused since `from`.
#[must_use]
pub fn macos_lost_since(from: u64) -> Statement {
    Statement {
        sql: "SELECT kind, number, outcome, finished_at FROM macos_jobs WHERE outcome IN \
              ('lost', 'unassigned', 'refused') AND finished_at >= ?1 ORDER BY finished_at DESC \
              LIMIT 10",
        params: vec![json!(from)],
    }
}

/// The `p`th percentile of `sorted`, nearest rank.
#[must_use]
pub fn percentile(sorted: &[u32], p: u32) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (u64::from(p) * u64::try_from(sorted.len()).unwrap_or(u64::MAX)).div_ceil(100);
    let index = usize::try_from(rank.saturating_sub(1)).unwrap_or(usize::MAX);
    sorted.get(index.min(sorted.len() - 1)).copied()
}

/// `n` as a `u32`, saturating.
fn n32(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `MainRun`'s deploys from `from`.
#[must_use]
pub fn deploys(history: &[Value], from: u64) -> Deploys {
    let mut kept: Vec<DeployRow> = history
        .iter()
        .filter(|h| num(h, "at") >= from)
        .map(|h| DeployRow {
            sha: text(h, "sha"),
            outcome: text(h, "outcome"),
            at: num(h, "at"),
        })
        .collect();
    let passed = n32(kept.iter().filter(|d| d.outcome == "passed").count());
    let total = n32(kept.len());
    kept.sort_by_key(|d| std::cmp::Reverse(d.at));
    kept.truncate(5);
    Deploys {
        passed,
        total,
        recent: kept,
    }
}

/// The merge queue, with the waits of recent merges.
#[must_use]
pub fn queue(q: &crate::change::queue::MergeQueue, waits: Vec<u32>) -> Queue {
    let mut sorted = waits.clone();
    sorted.sort_unstable();
    Queue {
        depth: n32(q.waiting.len()),
        merging: q.merging.as_ref().map(|e| e.number),
        p50: percentile(&sorted, 50),
        max: sorted.last().copied(),
        waits,
    }
}

fn since_latest(events: &[crate::events::Event], i: usize, kind: &str) -> Option<u32> {
    let e = &events[i];
    let prior = events[..i]
        .iter()
        .rev()
        .find(|p| p.kind == kind && p.change == e.change)?;
    Some(n32(e.at.saturating_sub(prior.at) / 60))
}

/// Whole minutes each merge came after its latest approval.
#[must_use]
pub fn merge_waits(events: &[crate::events::Event]) -> Vec<u32> {
    (0..events.len())
        .filter(|&i| events[i].kind == "change.merged")
        .filter_map(|i| since_latest(events, i, "change.approved"))
        .collect()
}

/// Whole minutes each owner approval came after its latest check.
#[must_use]
pub fn approval_waits(events: &[crate::events::Event], owner_approved: &BTreeSet<u32>) -> Vec<u32> {
    (0..events.len())
        .filter(|&i| {
            events[i].kind == "change.approved"
                && events[i]
                    .change
                    .is_some_and(|c| owner_approved.contains(&c))
        })
        .filter_map(|i| since_latest(events, i, "change.checked"))
        .collect()
}

/// What kind of approver `approver` names.
#[must_use]
pub fn approver_kind(approver: &str) -> &'static str {
    if approver.starts_with("window:") {
        "window"
    } else if approver.starts_with("mandate:") {
        "mandate"
    } else if approver.starts_with("policy:") {
        "policy"
    } else {
        "owner"
    }
}

/// Merges per day, lead time and approvers, from `merged_since`'s rows.
#[must_use]
pub fn throughput(rows: &[Value], from: u64, now: u64) -> Throughput {
    let first = from / DAY * DAY;
    let last = now / DAY * DAY;
    let mut days = Vec::new();
    let mut day = first;
    while day <= last {
        let merged = n32(rows
            .iter()
            .filter(|r| (day..day + DAY).contains(&num(r, "updated")))
            .count());
        days.push(Day { day, merged });
        day += DAY;
    }
    let mut leads: Vec<u32> = rows
        .iter()
        .map(|r| n32(num(r, "updated").saturating_sub(num(r, "created")) / 60))
        .collect();
    leads.sort_unstable();
    let mut by = By {
        owner: 0,
        window: 0,
        mandate: 0,
        policy: 0,
    };
    for r in rows {
        match approver_kind(&text(r, "approver")) {
            "window" => by.window += 1,
            "mandate" => by.mandate += 1,
            "policy" => by.policy += 1,
            _ => by.owner += 1,
        }
    }
    Throughput {
        days,
        lead_p50: percentile(&leads, 50),
        by,
    }
}

/// The newest ten failed code-check and lost macOS jobs.
#[must_use]
pub fn jobs(failed: &[Value], macos: &[Value]) -> Vec<JobRow> {
    let mut all: Vec<JobRow> = failed
        .iter()
        .map(|r| JobRow {
            name: text(r, "name"),
            change: r.get("change").and_then(Value::as_u64).map(n32),
            status: text(r, "status"),
            at: num(r, "updated"),
        })
        .chain(macos.iter().map(|r| JobRow {
            name: text(r, "kind"),
            change: r.get("number").and_then(Value::as_u64).map(n32),
            status: text(r, "outcome"),
            at: num(r, "finished_at"),
        }))
        .collect();
    all.sort_by_key(|j| std::cmp::Reverse(j.at));
    all.truncate(10);
    all
}

/// Assembles the answer.
#[must_use]
pub fn ops(
    range: u64,
    deploys: Deploys,
    queue: Queue,
    approvals: Approvals,
    throughput: Throughput,
    providers: Vec<Value>,
    jobs: Vec<JobRow>,
) -> Ops {
    Ops {
        range,
        deploys,
        queue,
        approvals,
        throughput,
        providers,
        jobs,
        note: (range == 30).then(|| {
            "Queue and approval waits cover the last 7 days: the event log keeps no more."
                .to_string()
        }),
    }
}
