//! A macOS job's attempts: placing, settling, moving, waiting, giving up, rows and events.

use serde_json::json;

use super::{
    GIVE_UP, Health, Job, Kind, Outcome, Placed, Provider, ProviderHealth, Settled, Usage,
    WAIT_RETRY, config, dispatch_failed, dispatched, ended_as, health_event, moved_away, observed,
    place, record, settle, usage_of,
};

const T: u64 = 1_794_480_000;
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn placed(job: &Job, now: u64) -> (Job, Placed) {
    place(
        job,
        &config().expect("parses"),
        &Health::new(),
        &Usage::new(),
        now,
    )
}

#[test]
fn a_job_runs_on_github_first_and_its_result_ends_it() {
    let j = Job::new(Kind::Check, 300, SHA, T, None, Vec::new());
    let (j, p) = placed(&j, T);
    assert_eq!(
        p,
        Placed::Run {
            provider: Provider::Github,
            probe: false
        }
    );
    let j = dispatched(
        &j,
        "43322097264",
        Some("https://github.com/r/actions/runs/1"),
    );
    let j = observed(&j, Outcome::Running, T + 5);
    assert_eq!(settle(&j).1, Settled::Pending);
    let j = observed(&j, Outcome::Passed, T + 170);
    let (j, s) = settle(&j);
    assert_eq!(
        s,
        Settled::Ran {
            provider: Provider::Github,
            passed: true
        }
    );
    assert_eq!(placed(&j, T + 200).1, Placed::Nothing, "it ended");
    let a = &j.attempts[0];
    assert_eq!((a.started_at, a.finished_at), (Some(T + 5), Some(T + 170)));
    let row = record(&j, a);
    assert_eq!(row.params[7], json!("passed"));
    assert_eq!(row.params[11], json!(3), "minutes, rounded up");
}

#[test]
fn an_unassigned_job_moves_to_bitrise_then_buildkite_then_waits() {
    let j = Job::new(Kind::Archive, 6, SHA, T, None, Vec::new());
    let (j, _) = placed(&j, T);
    let j = observed(&j, Outcome::Unassigned, T + 600);
    let (j, s) = settle(&j);
    assert_eq!(
        s,
        Settled::Moved {
            from: Provider::Github,
            why: "github gave no machine".into()
        }
    );
    let (j, p) = placed(&j, T + 600);
    assert!(matches!(
        p,
        Placed::Run {
            provider: Provider::Bitrise,
            ..
        }
    ));
    let j = dispatch_failed(&j, "Bitrise answered 500", T + 601);
    let j = dispatch_failed(&j, "Bitrise answered 500", T + 602);
    assert_eq!(settle(&j).1, Settled::Pending, "two failures retry");
    let j = dispatch_failed(&j, "Bitrise answered 500", T + 603);
    let (j, s) = settle(&j);
    assert_eq!(
        s,
        Settled::Moved {
            from: Provider::Bitrise,
            why: "bitrise refused the job: Bitrise answered 500".into()
        }
    );
    let (j, p) = placed(&j, T + 603);
    assert!(matches!(
        p,
        Placed::Run {
            provider: Provider::Buildkite,
            ..
        }
    ));
    let j = ended_as(
        &j,
        Outcome::Refused,
        "its pipeline steps differ from main's",
        T + 604,
    );
    let (j, _) = settle(&j);
    let (j, p) = placed(&j, T + 604);
    assert_eq!(
        p,
        Placed::Wait {
            why: "macOS (github: tried; bitrise: tried; buildkite: tried)".into()
        }
    );
    assert_eq!(
        placed(&j, T + 605).1,
        Placed::Nothing,
        "until its wait is over"
    );
    let (_, p) = placed(&j, T + 604 + WAIT_RETRY);
    assert!(
        matches!(
            p,
            Placed::Run {
                provider: Provider::Github,
                ..
            }
        ),
        "a new round from the order's start"
    );
}

#[test]
fn an_archive_gives_up_after_two_hours_but_a_check_waits_on() {
    let tried = vec![Provider::Github, Provider::Bitrise, Provider::Buildkite];
    let archive = Job::new(Kind::Archive, 6, SHA, T, None, tried.clone());
    let (_, p) = placed(&archive, T + GIVE_UP);
    assert!(matches!(p, Placed::GaveUp { .. }));
    let check = Job::new(Kind::Check, 300, SHA, T, None, tried);
    let (_, p) = placed(&check, T + GIVE_UP);
    assert!(matches!(p, Placed::Wait { .. }));
}

#[test]
fn a_second_lost_machine_fails_the_job() {
    let j = Job::new(Kind::Check, 300, SHA, T, None, Vec::new());
    let (j, _) = placed(&j, T);
    let j = observed(&j, Outcome::Running, T + 5);
    let j = observed(&j, Outcome::Lost, T + 300);
    let (j, s) = settle(&j);
    assert!(matches!(s, Settled::Moved { .. }));
    let (j, _) = placed(&j, T + 300);
    let j = observed(&j, Outcome::Lost, T + 900);
    let (j, s) = settle(&j);
    assert!(matches!(s, Settled::Failed { .. }));
    assert_eq!(j.result.as_ref().map(|r| r.passed), Some(false));
}

#[test]
fn a_failure_that_ran_never_moves() {
    let j = Job::new(Kind::Archive, 5, SHA, T, None, Vec::new());
    let (j, _) = placed(&j, T);
    let j = observed(&j, Outcome::Failed, T + 19);
    let (j, s) = settle(&j);
    assert_eq!(
        s,
        Settled::Ran {
            provider: Provider::Github,
            passed: false
        }
    );
    assert_eq!(placed(&j, T + 20).1, Placed::Nothing);
}

#[test]
fn the_ci_window_skips_github_in_the_first_round_only() {
    let j = Job::new(Kind::Check, 300, SHA, T, None, vec![Provider::Github]);
    let (j, p) = placed(&j, T);
    assert!(matches!(
        p,
        Placed::Run {
            provider: Provider::Bitrise,
            ..
        }
    ));
    assert_eq!(j.tried(), [Provider::Bitrise, Provider::Github]);
}

#[test]
fn a_forced_provider_runs_alone_and_fails_rather_than_moves() {
    let j = Job::new(
        Kind::Check,
        300,
        SHA,
        T,
        Some(Provider::Buildkite),
        Vec::new(),
    );
    let (j, p) = placed(&j, T);
    assert!(matches!(
        p,
        Placed::Run {
            provider: Provider::Buildkite,
            ..
        }
    ));
    let j = observed(&j, Outcome::Unassigned, T + 600);
    let (j, s) = settle(&j);
    assert!(matches!(s, Settled::Failed { .. }));
    assert_eq!(placed(&j, T + 601).1, Placed::Nothing);
}

#[test]
fn the_digest_names_moves_downs_and_providers_near_their_cap() {
    let cfg = config().expect("parses");
    let events = vec![
        (
            "ci.moved".to_string(),
            "!300 ios moved: github gave no machine".to_string(),
        ),
        ("change.pushed".to_string(), "!300 pushed".to_string()),
    ];
    let usage = Usage::from([(Provider::Bitrise, 360), (Provider::Buildkite, 10)]);
    assert_eq!(
        super::digest_lines(&events, &cfg, &usage),
        [
            "!300 ios moved: github gave no machine",
            "bitrise: 360 of 450 minutes this month"
        ]
    );
    let s = super::status(&cfg, &Health::new(), &usage, T);
    assert_eq!(s["providers"][1]["provider"], "bitrise");
    assert_eq!(s["providers"][1]["health"], "up");
    assert_eq!(s["providers"][1]["minutes"], 360);
    assert_eq!(s["providers"][0]["cap"], serde_json::Value::Null);
}

#[test]
fn usage_rows_and_health_events_read_plainly() {
    let rows = vec![
        json!({ "provider": "bitrise", "minutes": 42 }),
        json!({ "provider": "circle", "minutes": 1 }),
    ];
    assert_eq!(usage_of(&rows), Usage::from([(Provider::Bitrise, 42)]));
    let up = ProviderHealth::default();
    let down = moved_away(&moved_away(&up, T), T + 1);
    assert_eq!(
        health_event(Provider::Github, &up, &down).as_deref(),
        Some(
            "github is down for macOS jobs: 2 moved away within 30 minutes; a job probes it in 15 minutes"
        )
    );
    assert_eq!(
        health_event(Provider::Github, &down, &up).as_deref(),
        Some("github takes macOS jobs again: its probe ran")
    );
    assert_eq!(health_event(Provider::Github, &up, &up), None);
}
