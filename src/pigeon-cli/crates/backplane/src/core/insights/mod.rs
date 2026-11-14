//! The app's insights: cost and errors per hour, by source and kind, and the stacks the app
//! draws; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §7.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::Statement;
use crate::core::change::{BOUND, MERGE, MIRROR, RESTACK};

#[cfg(test)]
mod tests;

/// An hour, in seconds.
pub const HOUR: u64 = 60 * 60;
/// A day, in seconds.
pub const DAY: u64 = 24 * HOUR;
/// How long rows are kept, in seconds.
pub const KEEP: u64 = 90 * DAY;
/// The most hours one collection fills, oldest first.
pub const CATCH_UP: u64 = 48;

/// The sources, in the app's order.
pub const SOURCES: [&str; 4] = [CLOUDFLARE, GITHUB, BITRISE, BUILDKITE];
/// Cloudflare.
pub const CLOUDFLARE: &str = "cloudflare";
/// GitHub.
pub const GITHUB: &str = "github";
/// Buildkite.
pub const BUILDKITE: &str = "buildkite";
/// Bitrise, which takes macOS jobs GitHub gives no machine; see
/// `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §5.
pub const BITRISE: &str = "bitrise";
/// A macOS job a provider gave no machine.
pub const NO_MACHINE: &str = "No machine";
/// A macOS job whose machine went away.
pub const LOST_MACHINE: &str = "Lost machine";
/// A macOS job a provider refused.
pub const REFUSED: &str = "Refused";

/// GitHub's kinds of error.
pub const FAILED_CHECKS: &str = "Failed checks";
/// A check GitHub cancelled.
pub const CANCELLED_RUNS: &str = "Cancelled runs";
/// Buildkite's kinds of error.
pub const FAILED_SIGN: &str = "Failed sign steps";
/// A build cancelled on Buildkite.
pub const CANCELLED_BUILDS: &str = "Cancelled builds";
/// An agent Buildkite lost.
pub const AGENT_LOST: &str = "Agent lost";
/// Buildkite's kind of cost.
pub const MACOS_MINUTES: &str = "Hosted macOS minutes";

/// The unit prices, from `.config/insights.yaml`.
const PRICES: &str = include_str!("../../../../../../../.config/insights.yaml");

/// A metric's value for one hour, source and kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// The hour's start, in Unix seconds.
    pub hour: u64,
    /// `cost` or `errors`.
    pub metric: String,
    /// The source.
    pub source: String,
    /// The kind.
    pub kind: String,
    /// Dollars, or a count.
    pub value: f64,
}

fn row(hour: u64, metric: &str, source: &str, kind: &str, value: f64) -> Row {
    Row {
        hour,
        metric: metric.into(),
        source: source.into(),
        kind: kind.into(),
        value,
    }
}

/// The start of the hour holding `t`.
#[must_use]
pub fn hour_of(t: u64) -> u64 {
    t - t % HOUR
}

/// The hours to fill at `now`, after the newest hour filled: each complete hour since, at most
/// [`CATCH_UP`].
#[must_use]
pub fn due(newest: Option<u64>, now: u64) -> Vec<u64> {
    let last = hour_of(now).saturating_sub(HOUR);
    let first = newest.map_or(last, |n| n + HOUR);
    let first = first.max(last.saturating_sub((CATCH_UP - 1) * HOUR));
    if first > last {
        return Vec::new();
    }
    (0..=(last - first) / HOUR)
        .map(|i| first + i * HOUR)
        .collect()
}

/// A price from `.config/insights.yaml`: `key`'s value, or none when unset or `null`.
#[must_use]
pub fn price(yaml: &str, key: &str) -> Option<f64> {
    yaml.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == key))
        .and_then(|(_, v)| v.trim().parse().ok())
}

/// The hosted macOS rate, dollars a minute, when the owner has set one.
#[must_use]
pub fn macos_minute() -> Option<f64> {
    price(PRICES, "buildkite_macos_minute")
}

/// GitHub's error rows for `hour`, from D1's `jobs` rows (`name`, `status`) that finished in it.
#[must_use]
pub fn github(hour: u64, jobs: &[Value]) -> Vec<Row> {
    let (mut failed, mut cancelled) = (0u32, 0u32);
    for j in jobs {
        let name = j["name"].as_str().unwrap_or_default();
        if [BOUND, MIRROR, RESTACK, MERGE].contains(&name) {
            continue;
        }
        match j["status"].as_str() {
            Some("failed") => failed += 1,
            Some("cancelled") => cancelled += 1,
            _ => {}
        }
    }
    vec![
        row(hour, "errors", GITHUB, FAILED_CHECKS, f64::from(failed)),
        row(hour, "errors", GITHUB, CANCELLED_RUNS, f64::from(cancelled)),
    ]
}

/// Buildkite's rows for `hour`: errors from `ios_releases` rows (`state`, `error`,
/// `buildkite_build`) that ended in it, and cost from the sign jobs' `minutes` at `rate`.
#[must_use]
pub fn buildkite(hour: u64, releases: &[Value], minutes: f64, rate: Option<f64>) -> Vec<Row> {
    let (mut failed, mut cancelled, mut lost) = (0u32, 0u32, 0u32);
    for r in releases {
        if r["state"] != "failed" || r["buildkite_build"].is_null() {
            continue;
        }
        let why = r["error"].as_str().unwrap_or_default().to_ascii_lowercase();
        if why.contains("cancel") {
            cancelled += 1;
        } else if why.contains("lost") {
            lost += 1;
        } else {
            failed += 1;
        }
    }
    let mut rows = vec![
        row(hour, "errors", BUILDKITE, FAILED_SIGN, f64::from(failed)),
        row(
            hour,
            "errors",
            BUILDKITE,
            CANCELLED_BUILDS,
            f64::from(cancelled),
        ),
        row(hour, "errors", BUILDKITE, AGENT_LOST, f64::from(lost)),
    ];
    if let Some(rate) = rate {
        rows.push(row(hour, "cost", BUILDKITE, MACOS_MINUTES, minutes * rate));
    }
    rows
}

/// The rate of `source`'s macOS minutes, when the owner has set one: Bitrise's
/// `bitrise_macos_minute` once `.config/insights.yaml` holds it.
#[must_use]
pub fn rate_of(source: &str) -> Option<f64> {
    match source {
        BITRISE => price(PRICES, "bitrise_macos_minute"),
        BUILDKITE => macos_minute(),
        _ => None,
    }
}

/// macOS failover's rows for `hour`, from `macos_jobs` attempts (`provider`, `outcome`,
/// `minutes`) that ended in it: each provider's attempts that gave no machine, lost it or were
/// refused, and Bitrise's minutes at its rate. Buildkite's minutes join its sign steps' cost.
#[must_use]
pub fn macos(hour: u64, attempts: &[Value]) -> Vec<Row> {
    let mut rows = Vec::new();
    for source in [GITHUB, BITRISE, BUILDKITE] {
        let of = |outcome: &str| {
            let n = attempts
                .iter()
                .filter(|a| a["provider"] == source && a["outcome"] == outcome)
                .count();
            f64::from(u32::try_from(n).unwrap_or(u32::MAX))
        };
        rows.push(row(hour, "errors", source, NO_MACHINE, of("unassigned")));
        rows.push(row(hour, "errors", source, LOST_MACHINE, of("lost")));
        rows.push(row(hour, "errors", source, REFUSED, of("refused")));
    }
    if let Some(rate) = rate_of(BITRISE) {
        rows.push(row(
            hour,
            "cost",
            BITRISE,
            MACOS_MINUTES,
            minutes_of(attempts, BITRISE) * rate,
        ));
    }
    rows
}

/// `source`'s macOS minutes among `attempts`.
#[must_use]
pub fn minutes_of(attempts: &[Value], source: &str) -> f64 {
    attempts
        .iter()
        .filter(|a| a["provider"] == source)
        .filter_map(|a| a["minutes"].as_u64())
        .map(|m| f64::from(u32::try_from(m).unwrap_or(u32::MAX)))
        .sum()
}

/// Unix seconds of an RFC 3339 UTC time such as `2026-11-12T07:48:33.611Z`.
#[must_use]
pub fn unix(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+', '.']).next()?;
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    // Days from the civil date, after Howard Hinnant's algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3_600 + mm * 60 + ss).ok()
}

/// A Buildkite build's billed minutes: each job's run, rounded up to the minute.
#[must_use]
pub fn job_minutes(build: &Value) -> f64 {
    build["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|j| {
            let start = unix(j["started_at"].as_str()?)?;
            let end = unix(j["finished_at"].as_str()?)?;
            u32::try_from(end.saturating_sub(start).div_ceil(60)).ok()
        })
        .map(f64::from)
        .sum()
}

/// Writes `r`, replacing the hour's earlier value.
#[must_use]
pub fn upsert(r: &Row) -> Statement {
    Statement {
        sql: "INSERT OR REPLACE INTO insights (hour, metric, source, kind, value) \
              VALUES (?1, ?2, ?3, ?4, ?5)",
        params: vec![
            json!(r.hour),
            json!(r.metric),
            json!(r.source),
            json!(r.kind),
            json!(r.value),
        ],
    }
}

/// The newest hour filled.
#[must_use]
pub fn newest() -> Statement {
    Statement {
        sql: "SELECT MAX(hour) AS hour FROM insights",
        params: vec![],
    }
}

/// Drops rows older than [`KEEP`] before `now`.
#[must_use]
pub fn prune(now: u64) -> Statement {
    Statement {
        sql: "DELETE FROM insights WHERE hour < ?1",
        params: vec![json!(now.saturating_sub(KEEP))],
    }
}

/// The rows of `metric` from `from`.
#[must_use]
pub fn rows_since(metric: &str, from: u64) -> Statement {
    Statement {
        sql: "SELECT hour, metric, source, kind, value FROM insights \
              WHERE metric = ?1 AND hour >= ?2",
        params: vec![json!(metric), json!(from)],
    }
}

/// The code-check jobs that finished in `hour`.
#[must_use]
pub fn jobs_in(hour: u64) -> Statement {
    Statement {
        sql: "SELECT name, status FROM jobs WHERE updated >= ?1 AND updated < ?2 \
              AND status IN ('failed', 'cancelled')",
        params: vec![json!(hour), json!(hour + HOUR)],
    }
}

/// The iOS releases that ended in `hour`.
#[must_use]
pub fn releases_in(hour: u64) -> Statement {
    Statement {
        sql: "SELECT build, state, error, buildkite_build FROM ios_releases \
              WHERE updated_at >= ?1 AND updated_at < ?2 \
              AND state IN ('failed', 'uploaded', 'processing', 'delivered')",
        params: vec![json!(hour), json!(hour + HOUR)],
    }
}

/// The ranges the app asks for, in days.
pub const RANGES: [u64; 3] = [1, 7, 30];

/// The app's stacks of `metric` over `range` days to `now`, from `rows`: by source, or by kind
/// when `source` is named. One day is 24 hours; longer ranges are by day.
///
/// # Errors
///
/// When the range isn't 1, 7 or 30.
pub fn stacks(
    rows: &[Row],
    metric: &str,
    source: Option<&str>,
    range: u64,
    now: u64,
) -> Result<Value, String> {
    if !RANGES.contains(&range) {
        return Err(format!("range must be 1, 7 or 30, not {range}"));
    }
    let (step, count) = if range == 1 { (HOUR, 24) } else { (DAY, range) };
    let end = if range == 1 {
        hour_of(now) + HOUR
    } else {
        now - now % DAY + DAY
    };
    let start = end - step * count;
    let picked: Vec<&Row> = rows
        .iter()
        .filter(|r| r.metric == metric && r.hour >= start && r.hour < end)
        .filter(|r| source.is_none_or(|s| r.source == s))
        .collect();
    let part = |r: &Row| {
        if source.is_some() {
            r.kind.clone()
        } else {
            r.source.clone()
        }
    };
    let mut names: Vec<String> = Vec::new();
    for r in &picked {
        let n = part(r);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if source.is_none() {
        names.sort_by_key(|n| SOURCES.iter().position(|s| s == n).unwrap_or(SOURCES.len()));
    }
    let buckets: Vec<Value> = (0..count)
        .map(|i| {
            let at = start + i * step;
            let parts: Vec<Value> = names
                .iter()
                .map(|n| {
                    let v: f64 = picked
                        .iter()
                        .filter(|r| r.hour >= at && r.hour < at + step && part(r) == *n)
                        .fold(0.0, |a, r| a + r.value);
                    json!({ "name": n, "value": v })
                })
                .collect();
            json!({ "at": at, "parts": parts })
        })
        .collect();
    let legend: Vec<Value> = names
        .iter()
        .map(|n| {
            let total: f64 = picked
                .iter()
                .filter(|r| part(r) == *n)
                .fold(0.0, |a, r| a + r.value);
            json!({ "name": n, "total": total })
        })
        .collect();
    // From +0.0: `f64`'s `Sum` starts at -0.0, which the app would show as `$-0.00`.
    let total = picked.iter().fold(0.0, |a, r| a + r.value);
    Ok(json!({
        "metric": metric, "source": source, "range": range, "step": step,
        "total": total, "estimate": metric == "cost", "legend": legend, "buckets": buckets,
        "note": source.and_then(|s| note(metric, s)),
    }))
}

/// Why `source` has no rows, or only some kinds, for `metric`: what the app shows beside
/// its chart. See the ADR's 2026-11-13 amendment.
#[must_use]
pub fn note(metric: &str, source: &str) -> Option<&'static str> {
    match (metric, source) {
        (_, CLOUDFLARE) => Some("Not collected yet: Cloudflare waits for the observability token."),
        ("cost", GITHUB) => Some("GitHub Actions is free while the repository is public."),
        ("cost", BITRISE) => Some("Bitrise is free on Hobby with Pay-As-You-Go off."),
        ("errors", GITHUB) => Some("Dispatch errors aren't recorded yet."),
        _ => None,
    }
}
