//! The app's insights in the control plane: filled each hour by the cron, and read by the app's
//! routes; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §7.

use serde_json::{Value, json};
use worker::{Env, Fetch, Method, Result};

use super::platform::{d1, d1_rows, json_request, now, secret};
use crate::insights::{self, Row};

/// Fills each hour due: GitHub's and Buildkite's rows, each source failing alone. Cloudflare's
/// wait for `OBSERVABILITY_TOKEN`; see the ADR's 2026-11-12 amendment on step 4.
///
/// # Errors
///
/// When D1 fails.
pub async fn collect(env: &Env) -> Result<()> {
    let t = now();
    let newest = d1_rows(env, &insights::newest())
        .await?
        .first()
        .and_then(|r| r["hour"].as_u64());
    for hour in insights::due(newest, t) {
        let mut rows: Vec<Row> = Vec::new();
        match d1_rows(env, &insights::jobs_in(hour)).await {
            Ok(jobs) => rows.extend(insights::github(hour, &jobs)),
            Err(e) => worker::console_warn!("insights: GitHub's rows for {hour}: {e}"),
        }
        let attempts = match d1_rows(env, &crate::macos::ended_in(hour)).await {
            Ok(a) => {
                rows.extend(insights::macos(hour, &a));
                a
            }
            Err(e) => {
                worker::console_warn!("insights: macOS rows for {hour}: {e}");
                Vec::new()
            }
        };
        match d1_rows(env, &insights::releases_in(hour)).await {
            Ok(releases) => {
                let mut minutes = insights::minutes_of(&attempts, insights::BUILDKITE);
                for r in &releases {
                    let Some(number) = r["buildkite_build"].as_u64() else {
                        continue;
                    };
                    match buildkite_build(env, number).await {
                        Ok(b) => minutes += insights::job_minutes(&b),
                        Err(e) => worker::console_warn!("insights: Buildkite build {number}: {e}"),
                    }
                }
                rows.extend(insights::buildkite(
                    hour,
                    &releases,
                    minutes,
                    insights::macos_minute(),
                ));
            }
            Err(e) => worker::console_warn!("insights: Buildkite's rows for {hour}: {e}"),
        }
        let statements: Vec<_> = rows.iter().map(insights::upsert).collect();
        d1(env, &statements).await?;
    }
    d1(env, &[insights::prune(t)]).await
}

/// Buildkite signing build `number`, with its jobs' times.
async fn buildkite_build(env: &Env, number: u64) -> std::result::Result<Value, String> {
    let token = secret(env, "BUILDKITE_TOKEN").map_err(|e| e.to_string())?;
    let url = format!(
        "https://api.buildkite.com{}",
        crate::iosrelease::buildkite_path(Some(number))
    );
    let req = json_request(&url, Method::Get, None, Some(&token)).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&r.status_code()) {
        return Err(format!("Buildkite answered {}", r.status_code()));
    }
    r.json().await.map_err(|e| e.to_string())
}

async fn rows(env: &Env, metric: &str, days: u64) -> Result<Vec<Row>> {
    let from = now().saturating_sub(days * insights::DAY + insights::DAY);
    let raw = d1_rows(env, &insights::rows_since(metric, from)).await?;
    Ok(raw
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
}

/// `GET /v0/app/home`'s insights: today's cost and errors, by source and hour.
///
/// # Errors
///
/// When D1 fails.
pub async fn home(env: &Env) -> Result<Value> {
    let t = now();
    let mut tiles = json!({});
    for metric in ["cost", "errors"] {
        let r = rows(env, metric, 1).await?;
        tiles[metric] = insights::stacks(&r, metric, None, 1, t).unwrap_or(Value::Null);
    }
    Ok(json!({ "insights": tiles }))
}

/// `GET /v0/app/insights?metric=&source=&range=`: one source's stacks by kind.
///
/// # Errors
///
/// When D1 fails; a bad question is an answer saying why.
pub async fn detail(
    env: &Env,
    metric: &str,
    source: &str,
    range: u64,
) -> Result<std::result::Result<Value, String>> {
    if !["cost", "errors"].contains(&metric) {
        return Ok(Err("metric must be cost or errors".into()));
    }
    if !insights::SOURCES.contains(&source) {
        return Ok(Err(
            "source must be cloudflare, github, bitrise or buildkite".into(),
        ));
    }
    let r = rows(env, metric, range).await?;
    Ok(insights::stacks(&r, metric, Some(source), range, now()))
}
