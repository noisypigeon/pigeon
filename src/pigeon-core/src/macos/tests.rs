//! macOS failover: config, outcomes from recorded answers, verdicts, health and the next provider.

use serde_json::json;

use super::{
    Availability, Health, MAX_WAIT, Next, Outcome, PROBE_AFTER, Provider, ProviderHealth, Usage,
    Verdict, availability, bitrise, buildkite, config, github, minutes, month_start, moved_away,
    near_cap, next, parse, probing, ran, refused, same_config, tried_for_window, verdict,
};

const Q: u64 = 1_794_480_000;
const TEN: u64 = 600;

#[test]
fn the_built_in_config_is_githubs_then_bitrises_then_buildkites() {
    let c = config().expect("parses");
    assert_eq!(
        c.order,
        [Provider::Github, Provider::Bitrise, Provider::Buildkite]
    );
    assert_eq!(c.start_within, TEN);
    assert_eq!(c.cap(Provider::Github), None);
    assert_eq!(c.cap(Provider::Bitrise), Some(450));
    assert_eq!(c.cap(Provider::Buildkite), Some(150));
}

#[test]
fn a_config_with_an_unknown_missing_or_repeated_key_is_refused() {
    let good = "order: [bitrise]\nstart_within_minutes: 5\ngithub_monthly_minutes: null\nbitrise_monthly_minutes: 1\nbuildkite_monthly_minutes: 2\n";
    assert!(parse(good).is_ok());
    for (bad, why) in [
        (good.replace("[bitrise]", "[bitrise, bitrise]"), "twice"),
        (good.replace("[bitrise]", "[circle]"), "no provider"),
        (good.replace("[bitrise]", "[]"), "no provider"),
        (good.replace("5\n", "0\n"), "above 0"),
        (good.replace("bitrise_monthly_minutes: 1\n", ""), "missing"),
        (good.replace("2\n", "two\n"), "whole number"),
        (format!("{good}extra: 1\n"), "unknown"),
        (format!("{good}order: [github]\n"), "repeated"),
    ] {
        let err = parse(&bad).expect_err(&bad);
        assert!(err.contains(why), "{err} ∌ {why}");
    }
}

#[test]
fn githubs_jobs_of_2026_11_12_with_no_runner_are_unassigned() {
    // !300's `ios` (run 43319619862) and build 6's `ios.archive` (run 43323292545): cancelled
    // 15 minutes after creation, with no runner and no steps.
    let stuck = json!({
        "status": "completed", "conclusion": "cancelled",
        "runner_id": null, "runner_name": null, "steps": [],
        "created_at": "2026-11-12T10:00:00Z", "started_at": "2026-11-12T10:00:00Z",
    });
    let o = github(&stuck, "ios", Q, Q + 901, TEN);
    assert_eq!(o, Outcome::Unassigned);
    assert_eq!(
        verdict(Provider::Github, o, false),
        Verdict::Move {
            why: "github gave no machine".into()
        }
    );
    let queued = json!({ "status": "queued", "runner_id": null });
    assert_eq!(github(&queued, "ios", Q, Q + 599, TEN), Outcome::Waiting);
    assert_eq!(github(&queued, "ios", Q, Q + 600, TEN), Outcome::Unassigned);
    let zero = json!({ "status": "queued", "runner_id": 0 });
    assert_eq!(github(&zero, "ios", Q, Q + 600, TEN), Outcome::Unassigned);
}

#[test]
fn a_github_job_that_ran_is_a_result_and_never_moves() {
    // !300's rerun (run 43322097264): a runner in 5 seconds, passed in 2 minutes 45.
    let rerun = json!({
        "status": "completed", "conclusion": "success", "runner_id": 1_000_123,
        "steps": [{ "name": "ios", "status": "completed", "conclusion": "success" }],
    });
    assert_eq!(github(&rerun, "ios", Q, Q + 170, TEN), Outcome::Passed);
    // Build 5's archive (run 43316598157): ran and failed in 19 seconds.
    let build5 = json!({
        "status": "completed", "conclusion": "failure", "runner_id": 1_000_456,
        "steps": [{ "name": "ios.archive", "status": "completed", "conclusion": "failure" }],
    });
    let o = github(&build5, "ios.archive", Q, Q + 19, TEN);
    assert_eq!(o, Outcome::Failed);
    assert_eq!(
        verdict(Provider::Github, o, false),
        Verdict::Done { passed: false }
    );
    let timed_out = json!({ "status": "completed", "conclusion": "timed_out", "runner_id": 7 });
    assert_eq!(github(&timed_out, "ios", Q, Q + 3600, TEN), Outcome::Failed);
    let running = json!({ "status": "in_progress", "runner_id": 7 });
    assert_eq!(github(&running, "ios", Q, Q + 3000, TEN), Outcome::Running);
}

#[test]
fn a_github_job_cancelled_on_its_runner_is_lost_unless_our_step_finished() {
    let lost = json!({
        "status": "completed", "conclusion": "cancelled", "runner_id": 7,
        "steps": [{ "name": "ios", "status": "in_progress", "conclusion": null }],
    });
    assert_eq!(github(&lost, "ios", Q, Q + 900, TEN), Outcome::Lost);
    let finished = json!({
        "status": "completed", "conclusion": "cancelled", "runner_id": 7,
        "steps": [{ "name": "ios", "status": "completed", "conclusion": "failure" }],
    });
    assert_eq!(github(&finished, "ios", Q, Q + 900, TEN), Outcome::Failed);
}

#[test]
fn bitrise_builds_wait_run_finish_or_are_lost_by_status_and_worker() {
    let waiting = json!({ "status": 0, "started_on_worker_at": null, "is_on_hold": false });
    assert_eq!(bitrise(&waiting, Q, Q + 599, TEN), Outcome::Waiting);
    assert_eq!(bitrise(&waiting, Q, Q + 600, TEN), Outcome::Unassigned);
    let held = json!({ "status": 0, "started_on_worker_at": null, "is_on_hold": true });
    assert_eq!(bitrise(&held, Q, Q + 60, TEN), Outcome::Waiting);
    assert_eq!(bitrise(&held, Q, Q + 600, TEN), Outcome::Refused);
    let running = json!({ "status": 0, "started_on_worker_at": "2026-11-12T10:01:00Z" });
    assert_eq!(bitrise(&running, Q, Q + 3000, TEN), Outcome::Running);
    assert_eq!(bitrise(&json!({ "status": 1 }), Q, Q, TEN), Outcome::Passed);
    assert_eq!(bitrise(&json!({ "status": 2 }), Q, Q, TEN), Outcome::Failed);
    let aborted_started = json!({ "status": 3, "started_on_worker_at": "2026-11-12T10:01:00Z" });
    assert_eq!(bitrise(&aborted_started, Q, Q, TEN), Outcome::Lost);
    let aborted_waiting = json!({ "status": 3, "started_on_worker_at": null });
    assert_eq!(bitrise(&aborted_waiting, Q, Q, TEN), Outcome::Unassigned);
}

#[test]
fn buildkite_jobs_expired_or_agent_lost_move_and_failures_do_not() {
    let expired = json!({ "state": "expired", "started_at": null });
    assert_eq!(buildkite(&expired, Q, Q + 60, TEN), Outcome::Unassigned);
    let agent_lost = json!({
        "state": "failed", "started_at": "2026-11-12T10:01:00Z",
        "exit_status": -1, "signal_reason": "agent_lost",
    });
    assert_eq!(buildkite(&agent_lost, Q, Q + 900, TEN), Outcome::Lost);
    let failed =
        json!({ "state": "failed", "started_at": "2026-11-12T10:01:00Z", "exit_status": 1 });
    assert_eq!(buildkite(&failed, Q, Q + 900, TEN), Outcome::Failed);
    let passed =
        json!({ "state": "passed", "started_at": "2026-11-12T10:01:00Z", "exit_status": 0 });
    assert_eq!(buildkite(&passed, Q, Q + 900, TEN), Outcome::Passed);
    let cancelled = json!({ "state": "canceled", "started_at": "2026-11-12T10:01:00Z" });
    assert_eq!(buildkite(&cancelled, Q, Q + 900, TEN), Outcome::Lost);
    let cancelled_early = json!({ "state": "canceled", "started_at": null });
    assert_eq!(
        buildkite(&cancelled_early, Q, Q + 60, TEN),
        Outcome::Unassigned
    );
    let scheduled = json!({ "state": "scheduled", "started_at": null });
    assert_eq!(buildkite(&scheduled, Q, Q + 599, TEN), Outcome::Waiting);
    assert_eq!(buildkite(&scheduled, Q, Q + 600, TEN), Outcome::Unassigned);
    let running = json!({ "state": "running", "started_at": "2026-11-12T10:01:00Z" });
    assert_eq!(buildkite(&running, Q, Q + 3000, TEN), Outcome::Running);
}

#[test]
fn a_first_lost_moves_a_second_fails_and_refusals_count_dispatches() {
    assert!(matches!(
        verdict(Provider::Bitrise, Outcome::Lost, false),
        Verdict::Move { .. }
    ));
    assert!(matches!(
        verdict(Provider::Buildkite, Outcome::Lost, true),
        Verdict::Fail { .. }
    ));
    assert!(matches!(
        verdict(Provider::Github, Outcome::Refused, true),
        Verdict::Move { .. }
    ));
    assert_eq!(
        verdict(Provider::Github, Outcome::Running, true),
        Verdict::Wait
    );
    assert!(!refused(2) && refused(3));
    assert!(Outcome::Passed.ran() && Outcome::Failed.ran() && !Outcome::Lost.ran());
    assert!(Outcome::Unassigned.ended() && !Outcome::Waiting.ended());
}

#[test]
fn two_moves_in_thirty_minutes_mark_a_provider_down_then_a_probe_and_doubling() {
    let h = ProviderHealth::default();
    let h = moved_away(&h, Q);
    assert_eq!(
        availability(&h, Q),
        Availability::Up,
        "one move is not down"
    );
    let spaced = moved_away(&h, Q + 1800);
    assert_eq!(
        availability(&spaced, Q + 1800),
        Availability::Up,
        "30 minutes apart"
    );
    let h = moved_away(&h, Q + 1799);
    assert_eq!(
        availability(&h, Q + 1799),
        Availability::Down {
            until: Q + 1799 + PROBE_AFTER
        }
    );
    let t = Q + 1799 + PROBE_AFTER;
    assert_eq!(availability(&h, t), Availability::Probe);
    let h = probing(&h);
    assert_eq!(
        availability(&h, t),
        Availability::Down { until: t },
        "one probe at a time"
    );
    let failed = moved_away(&h, t + 600);
    assert_eq!(
        availability(&failed, t + 600),
        Availability::Down {
            until: t + 600 + 2 * PROBE_AFTER
        }
    );
    let mut d = failed;
    let mut now = t + 600;
    for _ in 0..6 {
        now = d.down.as_ref().expect("down").until;
        d = moved_away(&probing(&d), now);
    }
    assert_eq!(
        d.down.as_ref().expect("down").wait,
        MAX_WAIT,
        "doubles up to 2 hours"
    );
    assert_eq!(
        availability(&ran(&d), now),
        Availability::Up,
        "a probe that runs"
    );
}

fn health(p: Provider, h: ProviderHealth) -> Health {
    Health::from([(p, h)])
}

#[test]
fn next_follows_the_order_and_skips_tried_capped_and_down_providers() {
    let c = config().expect("parses");
    let none = Health::new();
    let unused = Usage::new();
    assert_eq!(
        next(&c, &none, &unused, &[], Q),
        Next::Run {
            provider: Provider::Github,
            probe: false
        }
    );
    assert_eq!(
        next(&c, &none, &unused, &tried_for_window(true), Q),
        Next::Run {
            provider: Provider::Bitrise,
            probe: false
        },
        "the CI window's ios skips GitHub"
    );
    assert_eq!(tried_for_window(false), Vec::<Provider>::new());
    let capped = Usage::from([(Provider::Bitrise, 450)]);
    assert_eq!(
        next(&c, &none, &capped, &[Provider::Github], Q),
        Next::Run {
            provider: Provider::Buildkite,
            probe: false
        }
    );
    let down = moved_away(&moved_away(&ProviderHealth::default(), Q), Q + 1);
    let h = health(Provider::Github, down.clone());
    assert_eq!(
        next(&c, &h, &unused, &[], Q + 2),
        Next::Run {
            provider: Provider::Bitrise,
            probe: false
        }
    );
    assert_eq!(
        next(&c, &h, &unused, &[], Q + 1 + PROBE_AFTER),
        Next::Run {
            provider: Provider::Github,
            probe: true
        }
    );
    let all = Usage::from([(Provider::Bitrise, 450), (Provider::Buildkite, 151)]);
    assert_eq!(
        next(&c, &h, &all, &[], Q + 2),
        Next::Wait {
            why: "macOS (github: down for 15 more minutes; bitrise: 450 of 450 minutes this month; buildkite: 151 of 150 minutes this month)".into()
        }
    );
    let tried = [Provider::Github, Provider::Bitrise, Provider::Buildkite];
    assert_eq!(
        next(&c, &none, &unused, &tried, Q),
        Next::Wait {
            why: "macOS (github: tried; bitrise: tried; buildkite: tried)".into()
        }
    );
}

#[test]
fn minutes_round_up_months_start_in_utc_and_caps_warn_at_80_percent() {
    assert_eq!(minutes(Q, Q), 0);
    assert_eq!(minutes(Q, Q + 1), 1);
    assert_eq!(minutes(Q, Q + 180), 3);
    // 2026-11-13T03:00:00Z is in the month from 2026-11-01T00:00:00Z.
    assert_eq!(month_start(1_794_538_800), 1_793_491_200);
    assert_eq!(month_start(1_793_491_200), 1_793_491_200);
    assert_eq!(month_start(1_793_491_199), 1_790_812_800, "October");
    // 2028-03-01, after a leap day.
    assert_eq!(month_start(1_835_481_600 + 3600), 1_835_481_600);
    assert!(near_cap(360, Some(450)) && !near_cap(359, Some(450)));
    assert!(!near_cap(10_000, None));
}

#[test]
fn a_providers_config_must_be_mains_up_to_line_endings() {
    let ours = "steps:\n  - label: ios build\n";
    assert!(same_config("steps:\r\n  - label: ios build\r\n\n", ours));
    assert!(!same_config("steps:\n  - label: something else\n", ours));
}
