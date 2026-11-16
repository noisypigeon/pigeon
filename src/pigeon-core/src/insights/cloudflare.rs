//! Cloudflare's insights: a GraphQL Analytics query per dataset, the rows and prices from their
//! answers, and what the app's note says; see
//! `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md`, the 2026-11-15
//! amendment "Cloudflare is collected".

use serde_json::{Value, json};

use super::{CLOUDFLARE, DAY, HOUR, Row, price, row};
use crate::control::Statement;

/// Cloudflare's GraphQL Analytics endpoint.
pub const ENDPOINT: &str = "https://api.cloudflare.com/client/v4/graphql";
/// The Worker secret the `observability` grant puts.
pub const TOKEN: &str = "OBSERVABILITY_TOKEN";

/// Invocations that threw.
pub const WORKER_EXCEPTIONS: &str = "Worker exceptions";
/// Invocations over their CPU limit.
pub const CPU_LIMIT: &str = "CPU limit exceeded";
/// Workers' requests and CPU.
pub const WORKERS: &str = "Workers";
/// Durable Objects' requests and duration.
pub const DURABLE_OBJECTS: &str = "Durable Objects";
/// R2's operations and D1's rows.
pub const R2_D1: &str = "R2 and D1";

/// Workers Paid list prices, in dollars per million units, in `.config/insights.yaml`'s form; kept
/// here while that file's policy admits only its own keys. See the ADR's 2026-11-15 amendment.
const PRICES: &str = "\
    cloudflare_workers_requests: 0.30\n\
    cloudflare_workers_cpu_ms: 0.02\n\
    cloudflare_do_requests: 0.15\n\
    cloudflare_do_gb_seconds: 12.50\n\
    cloudflare_d1_rows_read: 0.001\n\
    cloudflare_d1_rows_written: 1.00\n\
    cloudflare_r2_class_a: 4.50\n\
    cloudflare_r2_class_b: 0.36\n";
/// Durable Objects bill duration at 128 MB, in GB.
const DO_GB: f64 = 0.125;
/// A million.
const MILLION: f64 = 1_000_000.0;
/// The longest failure text kept.
const WHY_MAX: usize = 400;

/// One GraphQL dataset, queried alone so a field the API refuses fails only it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dataset {
    /// `workersInvocationsAdaptive`.
    Workers,
    /// `durableObjectsInvocationsAdaptiveGroups`.
    DoInvocations,
    /// `durableObjectsPeriodicGroups`.
    DoPeriodic,
    /// `d1AnalyticsAdaptiveGroups`.
    D1,
    /// `r2OperationsAdaptiveGroups`.
    R2,
}

impl Dataset {
    /// Every dataset, in query order.
    pub const ALL: [Self; 5] = [
        Self::Workers,
        Self::DoInvocations,
        Self::DoPeriodic,
        Self::D1,
        Self::R2,
    ];

    /// The GraphQL field.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::Workers => "workersInvocationsAdaptive",
            Self::DoInvocations => "durableObjectsInvocationsAdaptiveGroups",
            Self::DoPeriodic => "durableObjectsPeriodicGroups",
            Self::D1 => "d1AnalyticsAdaptiveGroups",
            Self::R2 => "r2OperationsAdaptiveGroups",
        }
    }

    /// The time dimension its filter takes.
    const fn time(self) -> &'static str {
        match self {
            Self::DoPeriodic => "datetimeMinute",
            Self::D1 => "datetimeHour",
            _ => "datetime",
        }
    }

    /// What it selects.
    const fn selection(self) -> &'static str {
        match self {
            Self::Workers => "sum { requests cpuTimeUs } dimensions { status }",
            Self::DoInvocations => "sum { requests }",
            Self::DoPeriodic => "sum { activeTime }",
            Self::D1 => "sum { rowsRead rowsWritten }",
            Self::R2 => "sum { requests } dimensions { actionType }",
        }
    }
}

/// `d`'s query, over `$from` to `$to` on account `$a`.
#[must_use]
pub fn query(d: Dataset) -> String {
    format!(
        "query($a: string!, $from: Time!, $to: Time!) {{ viewer {{ accounts(filter: \
         {{accountTag: $a}}) {{ rows: {field}(limit: 10000, filter: {{{t}_geq: $from, {t}_lt: \
         $to}}) {{ {sel} }} }} }} }}",
        field = d.field(),
        t = d.time(),
        sel = d.selection(),
    )
}

/// The request body for `d` over `hour` on `account`.
#[must_use]
pub fn body(d: Dataset, account: &str, hour: u64) -> Value {
    json!({
        "query": query(d),
        "variables": { "a": account, "from": rfc3339(hour), "to": rfc3339(hour + HOUR) },
    })
}

/// An answer's groups, or why there are none: GraphQL's errors, or no such account.
///
/// # Errors
///
/// When the answer carries errors or lacks the account.
pub fn groups(answer: &Value) -> Result<Vec<Value>, String> {
    if let Some(errors) = answer["errors"].as_array().filter(|e| !e.is_empty()) {
        let said: Vec<&str> = errors
            .iter()
            .map(|e| {
                e["message"]
                    .as_str()
                    .unwrap_or("an error without a message")
            })
            .collect();
        return Err(said.join("; "));
    }
    let accounts = answer["data"]["viewer"]["accounts"]
        .as_array()
        .ok_or("the answer has no accounts")?;
    let first = accounts.first().ok_or("the token sees no such account")?;
    Ok(first["rows"].as_array().cloned().unwrap_or_default())
}

/// One hour's usage, summed from the datasets that answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    /// Workers requests.
    pub requests: f64,
    /// Workers CPU, in milliseconds.
    pub cpu_ms: f64,
    /// Invocations that threw.
    pub exceptions: f64,
    /// Invocations over their CPU limit.
    pub cpu_limit: f64,
    /// Durable Objects requests.
    pub do_requests: f64,
    /// Durable Objects duration, in GB-seconds.
    pub do_gb_seconds: f64,
    /// D1 rows read.
    pub d1_read: f64,
    /// D1 rows written.
    pub d1_written: f64,
    /// R2 Class A operations.
    pub r2_a: f64,
    /// R2 Class B operations.
    pub r2_b: f64,
}

/// R2's Class B operations; see Cloudflare's R2 pricing.
const R2_CLASS_B: [&str; 8] = [
    "HeadBucket",
    "HeadObject",
    "GetObject",
    "UsageSummary",
    "GetBucketEncryption",
    "GetBucketLocation",
    "GetBucketCors",
    "GetBucketLifecycleConfiguration",
];
/// R2's free operations.
const R2_FREE: [&str; 3] = ["DeleteObject", "DeleteBucket", "AbortMultipartUpload"];

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

impl Usage {
    /// Adds `d`'s groups.
    pub fn add(&mut self, d: Dataset, groups: &[Value]) {
        for g in groups {
            let sum = &g["sum"];
            match d {
                Dataset::Workers => {
                    let n = num(&sum["requests"]);
                    self.requests += n;
                    self.cpu_ms += num(&sum["cpuTimeUs"]) / 1000.0;
                    match g["dimensions"]["status"].as_str() {
                        Some("scriptThrewException") => self.exceptions += n,
                        Some("exceededResources") => self.cpu_limit += n,
                        _ => {}
                    }
                }
                Dataset::DoInvocations => self.do_requests += num(&sum["requests"]),
                Dataset::DoPeriodic => {
                    self.do_gb_seconds += num(&sum["activeTime"]) / MILLION * DO_GB;
                }
                Dataset::D1 => {
                    self.d1_read += num(&sum["rowsRead"]);
                    self.d1_written += num(&sum["rowsWritten"]);
                }
                Dataset::R2 => {
                    let n = num(&sum["requests"]);
                    let action = g["dimensions"]["actionType"].as_str().unwrap_or_default();
                    if R2_CLASS_B.contains(&action) {
                        self.r2_b += n;
                    } else if !R2_FREE.contains(&action) {
                        self.r2_a += n;
                    }
                }
            }
        }
    }
}

/// `amount` units at `key`'s price per million, or none when the price is unset.
fn priced(prices: &str, key: &str, amount: f64) -> Option<f64> {
    price(prices, key).map(|p| amount / MILLION * p)
}

/// Sums the parts that have a price; none when no part does.
fn total(parts: &[Option<f64>]) -> Option<f64> {
    parts.iter().flatten().copied().reduce(|a, b| a + b)
}

/// `hour`'s rows from `u`, for the datasets in `ok` alone.
#[must_use]
pub fn rows(hour: u64, u: &Usage, ok: &[Dataset]) -> Vec<Row> {
    rows_priced(hour, u, ok, PRICES)
}

fn rows_priced(hour: u64, u: &Usage, ok: &[Dataset], prices: &str) -> Vec<Row> {
    let has = |d: Dataset| ok.contains(&d);
    let mut out = Vec::new();
    let mut cost = |kind: &str, v: Option<f64>| {
        if let Some(v) = v {
            out.push(row(hour, "cost", CLOUDFLARE, kind, v));
        }
    };
    if has(Dataset::Workers) {
        cost(
            WORKERS,
            total(&[
                priced(prices, "cloudflare_workers_requests", u.requests),
                priced(prices, "cloudflare_workers_cpu_ms", u.cpu_ms),
            ]),
        );
    }
    if has(Dataset::DoInvocations) || has(Dataset::DoPeriodic) {
        cost(
            DURABLE_OBJECTS,
            total(&[
                priced(prices, "cloudflare_do_requests", u.do_requests),
                priced(prices, "cloudflare_do_gb_seconds", u.do_gb_seconds),
            ]),
        );
    }
    if has(Dataset::D1) || has(Dataset::R2) {
        cost(
            R2_D1,
            total(&[
                priced(prices, "cloudflare_d1_rows_read", u.d1_read),
                priced(prices, "cloudflare_d1_rows_written", u.d1_written),
                priced(prices, "cloudflare_r2_class_a", u.r2_a),
                priced(prices, "cloudflare_r2_class_b", u.r2_b),
            ]),
        );
    }
    if has(Dataset::Workers) {
        out.push(row(
            hour,
            "errors",
            CLOUDFLARE,
            WORKER_EXCEPTIONS,
            u.exceptions,
        ));
        out.push(row(hour, "errors", CLOUDFLARE, CPU_LIMIT, u.cpu_limit));
    }
    out
}

/// A failure's text, cut to a length the note can show.
#[must_use]
pub fn why(d: Dataset, said: &str) -> String {
    let mut s = format!("{}: {said}", d.field());
    if s.len() > WHY_MAX {
        let mut end = WHY_MAX;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

/// What the newest collection did, as the note reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collected {
    /// The Worker has no `OBSERVABILITY_TOKEN`.
    NoToken,
    /// The token is present, and no collection has finished.
    NotYet,
    /// The newest collection, at `at`, failed for some datasets or all.
    Failed {
        /// When it ran.
        at: u64,
        /// The datasets that failed, and Cloudflare's words.
        why: String,
    },
    /// The newest collection, at `at`, read every dataset.
    Ok {
        /// When it ran.
        at: u64,
    },
}

impl Collected {
    /// From whether the token is present and the source's `insights_sources` row.
    #[must_use]
    pub fn from(token: bool, record: Option<&Value>) -> Self {
        if !token {
            return Self::NoToken;
        }
        let Some(r) = record else {
            return Self::NotYet;
        };
        let at = r["at"].as_u64().unwrap_or_default();
        match r["failed"].as_str().filter(|f| !f.is_empty()) {
            Some(why) => Self::Failed {
                at,
                why: why.to_string(),
            },
            None => Self::Ok { at },
        }
    }
}

/// Cloudflare's note for `metric`.
#[must_use]
pub fn note(metric: &str, c: &Collected) -> String {
    match c {
        Collected::NoToken => format!("Not collected: the Worker has no {TOKEN}."),
        Collected::NotYet => {
            "Token present; the first collection runs at :17 past the hour.".into()
        }
        Collected::Failed { at, why } => format!(
            "Token present; Cloudflare's analytics query failed at {} UTC: {why}",
            clock(*at)
        ),
        Collected::Ok { .. } if metric == "cost" => "An estimate: gross usage at list prices, \
             before the plan's included amounts; Containers and storage aren't counted yet."
            .into(),
        Collected::Ok { .. } => "5xx responses on the platform hostname aren't counted yet.".into(),
    }
}

/// The source's newest collection.
#[must_use]
pub fn record_of(source: &str) -> Statement {
    Statement {
        sql: "SELECT source, at, hour, failed FROM insights_sources WHERE source = ?1",
        params: vec![json!(source)],
    }
}

/// Records a collection at `at`: the newest `hour` filled, if any, and what failed.
#[must_use]
pub fn record(source: &str, at: u64, hour: Option<u64>, failed: Option<&str>) -> Statement {
    Statement {
        sql: "INSERT INTO insights_sources (source, at, hour, failed) VALUES (?1, ?2, ?3, ?4) \
              ON CONFLICT(source) DO UPDATE SET at = excluded.at, \
              hour = COALESCE(excluded.hour, insights_sources.hour), failed = excluded.failed",
        params: vec![json!(source), json!(at), json!(hour), json!(failed)],
    }
}

/// `t` as `HH:MM`.
fn clock(t: u64) -> String {
    format!("{:02}:{:02}", t % DAY / HOUR, t % HOUR / 60)
}

/// `t` as RFC 3339, in UTC.
#[must_use]
pub fn rfc3339(secs: u64) -> String {
    // Howard Hinnant's civil-from-days.
    let z = secs / DAY + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{}:{:02}Z",
        clock(secs),
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const HOUR0: u64 = 1_794_733_200; // 2026-11-15T09:00:00Z

    #[test]
    fn times_are_utc() {
        assert_eq!(rfc3339(HOUR0), "2026-11-15T09:00:00Z");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn each_dataset_queries_its_own_field_over_the_hour() {
        let b = body(Dataset::Workers, "acct", HOUR0);
        let q = b["query"].as_str().expect("a query");
        assert!(q.contains("rows: workersInvocationsAdaptive(limit: 10000"));
        assert!(q.contains("datetime_geq: $from, datetime_lt: $to"));
        assert!(q.contains("accountTag: $a"));
        assert_eq!(b["variables"]["from"], "2026-11-15T09:00:00Z");
        assert_eq!(b["variables"]["to"], "2026-11-15T10:00:00Z");
        assert!(query(Dataset::D1).contains("datetimeHour_geq"));
        assert!(query(Dataset::DoPeriodic).contains("datetimeMinute_lt"));
    }

    #[test]
    fn graphql_errors_and_a_missing_account_are_named() {
        let bad = json!({ "data": null, "errors": [
            { "message": "unknown field \"cpuTimeUs\"" }, { "message": "second" } ] });
        assert_eq!(
            groups(&bad).expect_err("errors"),
            "unknown field \"cpuTimeUs\"; second"
        );
        let none = json!({ "data": { "viewer": { "accounts": [] } }, "errors": null });
        assert_eq!(
            groups(&none).expect_err("none"),
            "the token sees no such account"
        );
        let ok = json!({ "data": { "viewer": { "accounts": [ { "rows": [ { "sum": {} } ] } ] } } });
        assert_eq!(groups(&ok).expect("rows").len(), 1);
    }

    fn recorded() -> Usage {
        let mut u = Usage::default();
        u.add(
            Dataset::Workers,
            &[
                json!({ "sum": { "requests": 1_000_000, "cpuTimeUs": 2_000_000_000 },
                        "dimensions": { "status": "success" } }),
                json!({ "sum": { "requests": 3, "cpuTimeUs": 0 },
                        "dimensions": { "status": "scriptThrewException" } }),
                json!({ "sum": { "requests": 2, "cpuTimeUs": 0 },
                        "dimensions": { "status": "exceededResources" } }),
            ],
        );
        u.add(
            Dataset::DoPeriodic,
            &[json!({ "sum": { "activeTime": 8_000_000_000_u64 } })],
        );
        u.add(
            Dataset::R2,
            &[
                json!({ "sum": { "requests": 1_000_000 }, "dimensions": { "actionType": "PutObject" } }),
                json!({ "sum": { "requests": 1_000_000 }, "dimensions": { "actionType": "GetObject" } }),
                json!({ "sum": { "requests": 5 }, "dimensions": { "actionType": "DeleteObject" } }),
            ],
        );
        u
    }

    #[test]
    fn a_recorded_hour_sums_usage_by_kind() {
        let u = recorded();
        assert_eq!(u.requests, 1_000_005.0);
        assert_eq!(u.cpu_ms, 2_000_000.0);
        assert_eq!((u.exceptions, u.cpu_limit), (3.0, 2.0));
        assert_eq!(u.do_gb_seconds, 1000.0);
        assert_eq!((u.r2_a, u.r2_b), (1_000_000.0, 1_000_000.0));
    }

    fn value(rows: &[Row], metric: &str, kind: &str) -> Option<f64> {
        rows.iter()
            .find(|r| r.metric == metric && r.kind == kind)
            .map(|r| r.value)
    }

    #[test]
    fn rows_price_usage_and_leave_out_datasets_that_failed() {
        let prices = "cloudflare_workers_requests: 0.30\ncloudflare_workers_cpu_ms: 0.02\n\
                      cloudflare_do_gb_seconds: 12.50\ncloudflare_r2_class_a: 4.50\n\
                      cloudflare_r2_class_b: 0.36\ncloudflare_do_requests: null\n";
        let u = recorded();
        let all = rows_priced(HOUR0, &u, &Dataset::ALL, prices);
        let near = |a: Option<f64>, b: f64| (a.expect("a row") - b).abs() < 1e-6;
        assert!(near(
            value(&all, "cost", WORKERS),
            1_000_005.0 / 1e6 * 0.30 + 2.0 * 0.02
        ));
        assert!(near(
            value(&all, "cost", DURABLE_OBJECTS),
            1000.0 / 1e6 * 12.50
        ));
        assert!(near(value(&all, "cost", R2_D1), 4.50 + 0.36));
        assert_eq!(value(&all, "errors", WORKER_EXCEPTIONS), Some(3.0));
        assert_eq!(value(&all, "errors", CPU_LIMIT), Some(2.0));
        assert!(
            all.iter()
                .all(|r| r.source == CLOUDFLARE && r.hour == HOUR0)
        );

        let some = rows_priced(HOUR0, &u, &[Dataset::R2], prices);
        assert_eq!(value(&some, "cost", WORKERS), None);
        assert_eq!(value(&some, "errors", WORKER_EXCEPTIONS), None);
        assert!(value(&some, "cost", R2_D1).is_some());

        assert!(
            rows_priced(HOUR0, &u, &Dataset::ALL, "")
                .iter()
                .all(|r| r.metric == "errors")
        );
    }

    #[test]
    fn the_shipped_prices_cover_every_unit() {
        for key in [
            "cloudflare_workers_requests",
            "cloudflare_workers_cpu_ms",
            "cloudflare_do_requests",
            "cloudflare_do_gb_seconds",
            "cloudflare_d1_rows_read",
            "cloudflare_d1_rows_written",
            "cloudflare_r2_class_a",
            "cloudflare_r2_class_b",
        ] {
            assert!(price(PRICES, key).is_some(), "{key}");
        }
    }

    #[test]
    fn the_note_names_what_actually_happened() {
        assert_eq!(Collected::from(false, None), Collected::NoToken);
        assert!(note("cost", &Collected::NoToken).contains("no OBSERVABILITY_TOKEN"));
        assert_eq!(Collected::from(true, None), Collected::NotYet);
        assert!(note("cost", &Collected::NotYet).starts_with("Token present"));

        let failed =
            json!({ "at": HOUR0 + 17 * 60, "failed": "d1AnalyticsAdaptiveGroups: unknown field" });
        let c = Collected::from(true, Some(&failed));
        assert_eq!(
            note("cost", &c),
            "Token present; Cloudflare's analytics query failed at 09:17 UTC: \
             d1AnalyticsAdaptiveGroups: unknown field"
        );

        let ok = json!({ "at": HOUR0, "failed": null });
        let c = Collected::from(true, Some(&ok));
        assert_eq!(c, Collected::Ok { at: HOUR0 });
        assert!(note("cost", &c).starts_with("An estimate"));
        assert!(note("errors", &c).contains("5xx"));
        assert!(!note("errors", &c).contains("waits"));
    }

    #[test]
    fn a_long_failure_is_cut_on_a_character() {
        let w = why(Dataset::R2, &"é".repeat(500));
        assert!(w.len() <= WHY_MAX + '…'.len_utf8());
        assert!(w.ends_with('…'));
    }
}
