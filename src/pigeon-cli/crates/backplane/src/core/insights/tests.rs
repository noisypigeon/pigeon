//! Insights: rows from each source, the hours due, prices, and the app's stacks.

use serde_json::json;

use super::{
    AGENT_LOST, BUILDKITE, CANCELLED_BUILDS, CANCELLED_RUNS, CATCH_UP, DAY, FAILED_CHECKS,
    FAILED_SIGN, GITHUB, HOUR, MACOS_MINUTES, Row, buildkite, due, github, job_minutes,
    macos_minute, price, stacks, unix,
};

const NOW: u64 = 1_800_003_600 + 25 * 60;

fn value(rows: &[Row], kind: &str) -> f64 {
    rows.iter()
        .find(|r| r.kind == kind)
        .map_or(-1.0, |r| r.value)
}

#[test]
fn github_counts_failed_and_cancelled_checks_but_not_the_control_planes_jobs() {
    let jobs = vec![
        json!({ "name": "test pigeon pigeon-backplane", "status": "failed" }),
        json!({ "name": "gate: test", "status": "failed" }),
        json!({ "name": "clippy", "status": "cancelled" }),
        json!({ "name": "bound", "status": "failed" }),
        json!({ "name": "mirror", "status": "failed" }),
        json!({ "name": "merge", "status": "cancelled" }),
    ];
    let rows = github(HOUR, &jobs);
    assert_eq!(value(&rows, FAILED_CHECKS), 2.0);
    assert_eq!(value(&rows, CANCELLED_RUNS), 1.0);
    assert!(
        rows.iter()
            .all(|r| r.source == GITHUB && r.metric == "errors")
    );
}

#[test]
fn buildkite_sorts_failures_by_cause_and_prices_minutes_only_with_a_rate() {
    let releases = vec![
        json!({ "state": "failed", "error": "Buildkite's build canceled", "buildkite_build": 3 }),
        json!({ "state": "failed", "error": "Buildkite's build failed", "buildkite_build": 2 }),
        json!({ "state": "failed", "error": "the agent was lost", "buildkite_build": 5 }),
        json!({ "state": "failed", "error": "the archive failed", "buildkite_build": null }),
        json!({ "state": "delivered", "error": null, "buildkite_build": 4 }),
    ];
    let rows = buildkite(HOUR, &releases, 10.0, Some(0.12));
    assert_eq!(value(&rows, CANCELLED_BUILDS), 1.0);
    assert_eq!(
        value(&rows, FAILED_SIGN),
        1.0,
        "a failure before Buildkite isn't its"
    );
    assert_eq!(value(&rows, AGENT_LOST), 1.0);
    assert!((value(&rows, MACOS_MINUTES) - 1.2).abs() < 1e-9);
    let unpriced = buildkite(HOUR, &releases, 10.0, None);
    assert!(unpriced.iter().all(|r| r.metric == "errors"));
}

#[test]
fn the_hours_due_are_the_complete_ones_since_the_newest_filled() {
    let last = NOW - NOW % HOUR - HOUR;
    assert_eq!(
        due(None, NOW),
        vec![last],
        "a first run fills the last hour"
    );
    assert_eq!(due(Some(last), NOW), Vec::<u64>::new(), "nothing twice");
    assert_eq!(due(Some(last - 2 * HOUR), NOW), vec![last - HOUR, last]);
    let far = due(Some(last - 1_000 * HOUR), NOW);
    assert_eq!(far.len() as u64, CATCH_UP);
    assert_eq!(far.last(), Some(&last));
}

#[test]
fn prices_are_read_by_key() {
    let yaml =
        "# buildkite_macos_minute: 9\ngithub_actions_minute: 0\nbuildkite_macos_minute: 0.12\n";
    assert_eq!(
        price(yaml, "buildkite_macos_minute"),
        Some(0.12),
        "a comment isn't a price"
    );
    assert_eq!(price(yaml, "github_actions_minute"), Some(0.0));
    assert_eq!(
        price("buildkite_macos_minute: null\n", "buildkite_macos_minute"),
        None
    );
    assert_eq!(macos_minute(), Some(0.12), "the repository's rate");
}

#[test]
fn stacks_bucket_by_hour_for_a_day_and_by_day_beyond() {
    let h = NOW - NOW % HOUR;
    let r = |hour, source: &str, kind: &str, value| Row {
        hour,
        metric: "errors".into(),
        source: source.into(),
        kind: kind.into(),
        value,
    };
    let rows = vec![
        r(h, GITHUB, FAILED_CHECKS, 2.0),
        r(h - HOUR, BUILDKITE, FAILED_SIGN, 1.0),
        r(h - 3 * DAY, GITHUB, CANCELLED_RUNS, 4.0),
    ];
    let day = stacks(&rows, "errors", None, 1, NOW).expect("a day");
    assert_eq!(day["buckets"].as_array().map(Vec::len), Some(24));
    assert_eq!(day["total"], 3.0, "only the last 24 hours");
    assert_eq!(
        day["legend"][0]["name"], GITHUB,
        "sources keep the app's order"
    );
    let last = &day["buckets"][23];
    assert_eq!(last["at"], h);
    assert_eq!(last["parts"][0]["value"], 2.0);

    let week = stacks(&rows, "errors", Some(GITHUB), 7, NOW).expect("a week");
    assert_eq!(week["buckets"].as_array().map(Vec::len), Some(7));
    assert_eq!(week["total"], 6.0);
    assert_eq!(week["step"], DAY);
    assert!(stacks(&rows, "errors", None, 3, NOW).is_err());
    assert_eq!(
        stacks(&rows, "cost", None, 1, NOW).expect("cost")["estimate"],
        true
    );
}

#[test]
fn buildkite_minutes_are_each_jobs_run_rounded_up() {
    assert_eq!(unix("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(unix("2026-11-12T07:48:33.611Z"), Some(1_794_469_713));
    assert_eq!(unix("not a time"), None);
    let build = json!({ "jobs": [
        { "started_at": "2026-11-12T07:40:00Z", "finished_at": "2026-11-12T07:48:33.611Z" },
        { "started_at": "2026-11-12T07:50:00Z", "finished_at": "2026-11-12T07:51:00Z" },
        { "started_at": null, "finished_at": null }
    ] });
    assert_eq!(job_minutes(&build), 10.0);
}
