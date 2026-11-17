//! The pure parts of routing a brief to a pool and dispatching its task; see `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`, its 2026-11-17 step 6 amendment.

use std::collections::BTreeMap;

use super::config::{Budget, Config, Pool};
use super::ledger::{self, Ledger};
use super::route::View;
use super::{dollars, micros};
use crate::sessions::Class;

/// The label prefix of a pool task's session.
pub const LABEL_PREFIX: &str = "pool:";
/// The model ids by class, as `tooling/session/models.json` holds them.
pub const MODELS: &str = include_str!("../../../../tooling/session/models.json");
/// Seconds past a task token's expiry after which its job counts as lost.
pub const LOST_AFTER: u64 = 15 * 60;

/// The label of brief `brief`'s `k`th pool task.
#[must_use]
pub fn label(brief: &str, k: u32) -> String {
    format!("pool:{brief}.{k}")
}

/// The id of brief `brief`'s `k`th pool task.
#[must_use]
pub fn task_id(brief: &str, k: u32) -> String {
    format!("{brief}.{k}")
}

/// The next `k` for `brief`, given every ticket label.
#[must_use]
pub fn next_k<'a>(labels: impl IntoIterator<Item = &'a str>, brief: &str) -> u32 {
    let prefix = format!("{LABEL_PREFIX}{brief}.");
    labels
        .into_iter()
        .filter_map(|l| l.strip_prefix(&prefix)?.parse::<u32>().ok())
        .max()
        .map_or(1, |k| k + 1)
}

/// The model id for `class`.
///
/// # Errors
///
/// When `models.json` has no string for the class.
pub fn model(class: Class) -> Result<String, String> {
    let missing = || {
        format!(
            "no model for `{}` in tooling/session/models.json",
            class.name()
        )
    };
    let v: serde_json::Value = serde_json::from_str(MODELS).map_err(|_| missing())?;
    v.get(class.name())
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(missing)
}

/// The live executor sessions: haiku or sonnet, not ended, not a pool task's.
#[must_use]
pub fn executors(tickets: &crate::tickets::Tickets, now: u64) -> u32 {
    let n = tickets
        .0
        .iter()
        .filter(|(label, t)| {
            t.expires > now
                && t.ended.is_none()
                && matches!(t.declared.model, Class::Haiku | Class::Sonnet)
                && !label.starts_with(LABEL_PREFIX)
        })
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `pool` as the router sees it for a task of `class` with `budget`.
#[must_use]
pub fn view(pool: &Pool, ledger: Option<&Ledger>, class: Class, budget: Budget, now: u64) -> View {
    let l = ledger.cloned().unwrap_or_else(|| ledger::new(pool, now));
    let l = ledger::roll(&l, pool, now);
    View {
        name: pool.name.clone(),
        harness: pool.harness.clone(),
        state: ledger::state(&l, pool, now),
        running: ledger::running(&l),
        concurrency: pool.concurrency,
        admits: ledger::admit_task(&l, pool, "probe", class, budget, now).is_ok(),
        cycle_end: l.cycle.end,
        headroom: ledger::headroom(&l),
    }
}

/// The task's budget: the class's, with the brief's overrides.
///
/// # Errors
///
/// When `pool_usd` is not a dollar amount, or an override is above its maximum.
pub fn budget(cfg: &Config, draft: &crate::brief::Draft) -> Result<Budget, String> {
    let usd = match draft.pool_usd.as_deref() {
        Some(t) => Some(micros(t).ok_or("`pool_usd` is not a dollar amount")?),
        None => None,
    };
    cfg.budget(draft.model, usd, draft.pool_minutes)
}

/// The URL of `pool`'s gateway on `host`.
#[must_use]
pub fn pool_url(host: &str, pool: &str) -> String {
    format!("https://{host}/v0/pool/{pool}")
}

/// When a task token opened at `opened` expires.
#[must_use]
pub fn token_expiry(opened: u64, budget: &Budget) -> u64 {
    opened + u64::from(budget.minutes) * 60
}

/// When a task opened at `opened` counts as lost.
#[must_use]
pub fn lost_at(opened: u64, budget: &Budget) -> u64 {
    token_expiry(opened, budget) + LOST_AFTER
}

/// The secrets a job is handed.
pub struct Secrets<'a> {
    /// The task's token.
    pub task_token: &'a str,
    /// The Access client id.
    pub access_id: &'a str,
    /// The Access client secret.
    pub access_secret: &'a str,
}

/// The job's environment.
#[must_use]
pub fn job_env(
    task: &str,
    url: &str,
    model: &str,
    brief: &serde_json::Value,
    budget: &Budget,
    s: &Secrets<'_>,
) -> BTreeMap<String, String> {
    use super::job as j;
    [
        (j::TASK, task.to_string()),
        (j::TOKEN, s.task_token.to_string()),
        (j::URL, url.to_string()),
        (j::ACCESS_ID, s.access_id.to_string()),
        (j::ACCESS_SECRET, s.access_secret.to_string()),
        (j::MODEL, model.to_string()),
        (j::BRIEF, brief.to_string()),
        (j::TURNS, budget.requests.to_string()),
        (j::USD, dollars(budget.usd)),
        (j::MINUTES, budget.minutes.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// What the Worker does with a finished task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Report the pushed head.
    Pushed {
        /// The head's commit.
        head: String,
    },
    /// Escalate to the planner.
    Escalate {
        /// The question.
        question: String,
        /// What was tried.
        tried: String,
    },
}

/// The action for a job's result, and the outcome the task closes with.
#[must_use]
pub fn action(p: Option<&super::job::Pooled>) -> (Action, String) {
    use super::job::Outcome;
    let escalate = |question: String, tried: &str| Action::Escalate {
        question,
        tried: tried.to_string(),
    };
    let Some(p) = p else {
        return (
            escalate("the pool task's job was lost".into(), ""),
            "failed".into(),
        );
    };
    match (p.outcome, &p.pushed) {
        (Outcome::Done, Some(h)) => (Action::Pushed { head: h.clone() }, "done".into()),
        (Outcome::Done, None) => (
            escalate("the pool task finished but pushed nothing".into(), &p.why),
            "failed".into(),
        ),
        (Outcome::Escalated, _) => {
            let field = |k: &str| {
                p.escalation
                    .as_ref()
                    .and_then(|e| e.get(k))
                    .and_then(serde_json::Value::as_str)
            };
            (
                escalate(
                    field("question").unwrap_or(&p.why).to_string(),
                    field("tried").unwrap_or(""),
                ),
                "escalated".into(),
            )
        }
        (Outcome::Stopped, _) => (
            escalate(format!("the pool task stopped: {}", p.why), ""),
            format!("stopped ({})", p.why),
        ),
        (Outcome::Failed, _) => (
            escalate(format!("the pool task failed: {}", p.why), ""),
            "failed".into(),
        ),
    }
}

#[cfg(test)]
mod tests;
