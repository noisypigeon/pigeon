//! The app's insights in the control plane: filled each hour by the cron, and read by the app's
//! routes; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §7.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use worker::{Env, Fetch, Method, Result};

use super::platform::{Settings, account, d1, d1_rows, json_request, now, secret};
use crate::insights::cloudflare::{self, Collected, Dataset, Usage};
use crate::insights::{self, CLOUDFLARE, Row, ops};

/// Fills each hour due: GitHub's, macOS's and Buildkite's rows, then Cloudflare's, each source
/// failing alone; see the ADR's 2026-11-15 amendment "Cloudflare is collected".
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
    if let Err(e) = collect_cloudflare(env, t).await {
        worker::console_warn!("insights: Cloudflare: {e}");
    }
    d1(env, &[insights::prune(t)]).await
}

/// Fills Cloudflare's due hours from GraphQL Analytics, a query per dataset, and records the run;
/// an hour fills with the datasets that answer. Nothing runs without the token.
async fn collect_cloudflare(env: &Env, t: u64) -> Result<()> {
    let Ok(token) = secret(env, cloudflare::TOKEN) else {
        return Ok(());
    };
    let newest = d1_rows(env, &cloudflare::record_of(CLOUDFLARE))
        .await?
        .first()
        .and_then(|r| r["hour"].as_u64());
    let due = insights::due(newest, t);
    if due.is_empty() {
        return Ok(());
    }
    let acct = match secret(env, "ARTIFACTS_TOKEN") {
        Ok(a) => account(&a).await,
        Err(e) => Err(e),
    };
    let acct = match acct {
        Ok(a) => a,
        Err(e) => {
            let why = format!("looking up the account: {e}");
            return d1(env, &[cloudflare::record(CLOUDFLARE, t, None, Some(&why))]).await;
        }
    };
    let mut filled = None;
    let mut failed = BTreeSet::new();
    for hour in due {
        let mut usage = Usage::default();
        let mut ok = Vec::new();
        for d in Dataset::ALL {
            match graphql(&token, &acct, d, hour).await {
                Ok(groups) => {
                    usage.add(d, &groups);
                    ok.push(d);
                }
                Err(e) => {
                    failed.insert(cloudflare::why(d, &e.replace(token.as_str(), "[token]")));
                }
            }
        }
        if ok.is_empty() {
            break;
        }
        let rows = cloudflare::rows(hour, &usage, &ok);
        let statements: Vec<_> = rows.iter().map(insights::upsert).collect();
        d1(env, &statements).await?;
        filled = Some(hour);
    }
    let failed = (!failed.is_empty()).then(|| failed.into_iter().collect::<Vec<_>>().join("; "));
    if let Some(why) = &failed {
        worker::console_warn!("insights: Cloudflare: {why}");
    }
    d1(
        env,
        &[cloudflare::record(CLOUDFLARE, t, filled, failed.as_deref())],
    )
    .await
}

/// Dataset `d`'s groups over `hour`, or Cloudflare's words for why not.
async fn graphql(
    token: &str,
    acct: &str,
    d: Dataset,
    hour: u64,
) -> std::result::Result<Vec<Value>, String> {
    let body = cloudflare::body(d, acct, hour);
    let req = json_request(cloudflare::ENDPOINT, Method::Post, Some(&body), Some(token))
        .map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    let v: Value = r
        .json()
        .await
        .map_err(|_| format!("Cloudflare answered {status}, not JSON"))?;
    let groups = cloudflare::groups(&v);
    if (200..300).contains(&status) {
        groups
    } else {
        Err(groups
            .err()
            .unwrap_or_else(|| format!("Cloudflare answered {status}")))
    }
}

/// What Cloudflare's newest collection did, for its note.
async fn collected(env: &Env) -> Collected {
    let token = secret(env, cloudflare::TOKEN).is_ok();
    let record = d1_rows(env, &cloudflare::record_of(CLOUDFLARE))
        .await
        .ok()
        .and_then(|r| r.into_iter().next());
    Collected::from(token, record.as_ref())
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
        tiles[metric] =
            insights::stacks(&r, metric, None, 1, t, &Collected::NoToken).unwrap_or(Value::Null);
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
    let cf = if source == CLOUDFLARE {
        collected(env).await
    } else {
        Collected::NoToken
    };
    Ok(insights::stacks(
        &r,
        metric,
        Some(source),
        range,
        now(),
        &cf,
    ))
}

/// The most pages of the event log one answer reads.
const OPS_PAGES: usize = 15;

/// The event log from `from`, a page at a time; what a failing read leaves is what it got.
async fn log_since(env: &Env, s: &Settings, from: u64) -> Vec<crate::events::Event> {
    let mut events = Vec::new();
    let mut after = 0;
    for _ in 0..OPS_PAGES {
        let body = json!({ "after": after });
        let Ok((_, v)) = super::mandates::to_instance(env, s, "/events/since", &body).await else {
            break;
        };
        let page = v["events"].as_array().cloned().unwrap_or_default();
        for e in &page {
            after = e["id"].as_u64().unwrap_or(after);
            if let Ok(e) = serde_json::from_value::<crate::events::Event>(e.clone())
                && e.at >= from
            {
                events.push(e);
            }
        }
        if page.len() < crate::events::PAGE || after >= v["last"].as_u64().unwrap_or_default() {
            break;
        }
    }
    events
}

/// `GET /v0/app/ops?range=`: deploys, queue and approval waits, throughput, providers and failed
/// jobs; each source failing alone, its panel empty.
///
/// # Errors
///
/// When building a request fails.
pub async fn ops(env: &Env, s: &Settings, range: u64) -> Result<Value> {
    let t = now();
    let from = t.saturating_sub(range * insights::DAY);

    let history = match json_request("https://main/status", Method::Get, None, None) {
        Ok(req) => match super::platform::call(env, &super::main_stub(env, &s.repo)?, req).await {
            Ok(mut r) => r.json::<Value>().await.unwrap_or_default(),
            Err(_) => Value::Null,
        },
        Err(_) => Value::Null,
    };
    let history = history["pipeline"]["history"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let queue = super::stages::queue(env, s).await;
    let events = log_since(env, s, from).await;
    let merged = d1_rows(env, &ops::merged_since(&s.instance, &s.repo, from))
        .await
        .unwrap_or_default();
    let failed = d1_rows(env, &ops::failed_jobs_since(&s.instance, &s.repo, from))
        .await
        .unwrap_or_default();
    let lost = d1_rows(env, &ops::macos_lost_since(from))
        .await
        .unwrap_or_default();

    let providers = match crate::macos::config() {
        Ok(cfg) => {
            let health = super::macos::health(env, s).await;
            let usage = super::macos::usage(env).await.unwrap_or_default();
            crate::macos::status(&cfg, &health, &usage, t)["providers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };

    let owner_approved: BTreeSet<u32> = merged
        .iter()
        .filter(|r| ops::approver_kind(r["approver"].as_str().unwrap_or_default()) == "owner")
        .filter_map(|r| r["number"].as_u64().and_then(|n| u32::try_from(n).ok()))
        .collect();
    let mut approval = ops::approval_waits(&events, &owner_approved);
    approval.sort_unstable();
    let approvals = ops::Approvals {
        count: u32::try_from(approval.len()).unwrap_or(u32::MAX),
        p50: ops::percentile(&approval, 50),
        p95: ops::percentile(&approval, 95),
    };

    let answer = ops::ops(
        range,
        ops::deploys(&history, from),
        ops::queue(&queue, ops::merge_waits(&events)),
        approvals,
        ops::throughput(&merged, from, t),
        providers,
        ops::jobs(&failed, &lost),
    );
    Ok(serde_json::to_value(answer)?)
}
