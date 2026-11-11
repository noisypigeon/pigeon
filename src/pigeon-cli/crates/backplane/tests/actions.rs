//! A change's code checks on GitHub Actions; see
//! `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §3–§5.

use backplane::actions::{
    Actions, Due, FIND_WITHIN, Finished, Mirror, POLL, dispatch_body, on_github,
};
use backplane::changes::Check;
use serde_json::{Value, json};

const A: &str = "0123456789abcdef0123456789abcdef01234567";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn argv(s: &str) -> Vec<String> {
    s.split(' ').map(str::to_string).collect()
}

fn check(name: &str, cmd: &str) -> Check {
    Check {
        name: name.into(),
        argv: argv(cmd),
    }
}

/// Entry 0 on `A` for `fmt` and `test`, dispatched at 100.
fn dispatched() -> Actions {
    let mut a = Actions::default();
    a.queue(7, A, "fmt");
    a.queue(7, A, "test");
    assert_eq!(a.due(Mirror::Passed, 100), [Due::Dispatch(0)]);
    assert_eq!(a.dispatched(0, 100), ["fmt", "test"]);
    a
}

fn runs(title: &str) -> Value {
    json!({ "workflow_runs": [
        { "id": 1, "display_title": "!7 other (7-0123456789ab-9)", "html_url": "https://github.com/o/r/actions/runs/1" },
        { "id": 42, "display_title": title, "html_url": "https://github.com/o/r/actions/runs/42" },
    ]})
}

#[test]
fn only_code_checks_run_on_github() {
    for code in [
        "cargo fmt --all -- --check",
        "bash ../../tooling/ci/test-map.sh",
        "cargo clippy -p pigeon-backplane --all-targets --locked -- -D warnings",
        "cargo test -p pigeon-backplane --locked",
        "cargo build --release",
        "cargo check -p sentinel",
    ] {
        assert!(on_github(&argv(code)), "{code}");
    }
    for platform in [
        "cargo run -q --locked -p pigeon -- backplane run policy.check --root ../..",
        "cargo run -q --locked -p pigeon -- backplane run infra.validate --root ../..",
        "sh ../../spikes/1/run.sh",
        "bash ../../tooling/ci/other.sh",
        "fmt",
        "",
    ] {
        assert!(!on_github(&argv(platform)), "{platform}");
    }
}

#[test]
fn a_dispatch_waits_for_the_head_on_github_and_runs_mains_workflow() {
    let mut a = Actions::default();
    a.queue(7, A, "fmt");
    a.queue(7, A, "test");
    a.queue(7, A, "fmt");
    assert_eq!(a.dispatches.len(), 1, "one entry per head until dispatched");
    assert!(a.owns(A, "fmt") && !a.owns(B, "fmt") && !a.owns(A, "policy.check"));
    assert_eq!(a.due(Mirror::Open, 10), []);
    assert_eq!(
        a.view()[0]["waiting"],
        "the head isn't on GitHub yet",
        "{}",
        a.view()
    );
    assert_eq!(a.due(Mirror::Passed, 11), [Due::Dispatch(0)]);
    let body = dispatch_body(
        &a.dispatches[0],
        7,
        &[
            check("bound", ""),
            check("fmt", "cargo fmt --all -- --check"),
            check("test", "cargo test --workspace --locked"),
        ],
    );
    assert_eq!(body["ref"], "main", "always main's ci.yml");
    assert_eq!(body["inputs"]["sha"], A);
    assert_eq!(body["inputs"]["number"], "7");
    assert_eq!(body["inputs"]["dispatch"], "7-0123456789ab-1");
    let checks: Value =
        serde_json::from_str(body["inputs"]["checks"].as_str().expect("a string input"))
            .expect("JSON");
    assert_eq!(
        checks,
        json!([
            { "name": "fmt", "argv": ["cargo", "fmt", "--all", "--", "--check"] },
            { "name": "test", "argv": ["cargo", "test", "--workspace", "--locked"] },
        ])
    );
    assert_eq!(a.dispatched(0, 12), ["fmt", "test"]);
    a.queue(7, A, "clippy");
    assert_eq!(
        a.dispatches.len(),
        2,
        "a dispatched entry takes no more checks"
    );
    assert_eq!(a.dispatches[1].id, "7-0123456789ab-2");
}

#[test]
fn a_run_is_found_by_its_dispatch_id_and_dispatched_again_if_it_never_appears() {
    let mut a = dispatched();
    assert_eq!(a.due(Mirror::Passed, 100 + POLL - 1), []);
    assert_eq!(a.due(Mirror::Passed, 100 + POLL), [Due::Find(0)]);
    a.found(0, &runs("!7 x (7-0123456789ab-10)"), 130);
    assert_eq!(
        a.dispatches[0].run, None,
        "a longer ID that merely ends alike is not it"
    );
    a.found(0, &runs(&format!("!7 {A} (7-0123456789ab-1)")), 160);
    assert_eq!(a.dispatches[0].run, Some(42));
    assert_eq!(a.view()[0]["run"], "https://github.com/o/r/actions/runs/42");
    assert_eq!(a.due(Mirror::Passed, 160 + POLL), [Due::Poll(0)]);

    let mut lost = dispatched();
    lost.found(0, &runs("!7 x (7-other)"), 100 + FIND_WITHIN);
    let d = &lost.dispatches[0];
    assert_eq!(d.dispatched_at, None, "dispatched again");
    assert_eq!(d.id, "7-0123456789ab-2", "under a new ID");
    assert!(
        d.waiting
            .as_deref()
            .is_some_and(|w| w.contains("never appeared"))
    );
    assert_eq!(lost.due(Mirror::Passed, 100 + FIND_WITHIN + 59), []);
    assert_eq!(
        lost.due(Mirror::Passed, 100 + FIND_WITHIN + 60),
        [Due::Dispatch(0)]
    );
}

#[test]
fn finished_jobs_become_results_and_a_run_missing_a_check_fails_it() {
    let mut a = dispatched();
    a.found(0, &runs(&format!("!7 {A} (7-0123456789ab-1)")), 130);
    let jobs = json!({ "jobs": [
        { "id": 5, "name": "fmt", "status": "completed", "conclusion": "success" },
        { "id": 6, "name": "test", "status": "in_progress", "conclusion": null },
    ]});
    let run =
        json!({ "status": "in_progress", "html_url": "https://github.com/o/r/actions/runs/42" });
    assert_eq!(
        a.polled(0, &run, &jobs, 200),
        [Finished {
            name: "fmt".into(),
            ok: true,
            job: Some(5)
        }]
    );
    assert_eq!(
        a.polled(0, &run, &jobs, 230),
        [],
        "each result is applied once"
    );
    assert!(!a.owns(A, "fmt") && a.owns(A, "test"));
    let done = json!({ "status": "completed" });
    let failed = json!({ "jobs": [
        { "id": 5, "name": "fmt", "status": "completed", "conclusion": "success" },
        { "id": 6, "name": "test", "status": "completed", "conclusion": "cancelled" },
    ]});
    assert_eq!(
        a.polled(0, &done, &failed, 260),
        [Finished {
            name: "test".into(),
            ok: false,
            job: Some(6)
        }]
    );
    assert_eq!(a.due(Mirror::Passed, 10_000), [], "nothing more is due");

    let mut missing = dispatched();
    missing.found(0, &runs(&format!("!7 {A} (7-0123456789ab-1)")), 130);
    let none = json!({ "jobs": [] });
    assert_eq!(
        missing.polled(0, &json!({ "status": "queued" }), &none, 200),
        []
    );
    let failed = missing.polled(0, &done, &none, 230);
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|f| !f.ok && f.job.is_none()));
}

#[test]
fn github_failures_back_off_and_show_waiting() {
    let mut a = dispatched();
    for (at, wait) in [(130, 60), (190, 300), (490, 1800), (2290, 1800)] {
        assert_eq!(a.due(Mirror::Passed, at), [Due::Find(0)]);
        a.failed(
            0,
            "GitHub /repos/o/r/actions/workflows/ci.yml/runs: 503",
            at,
        );
        assert_eq!(a.dispatches[0].next_at, Some(at + wait));
    }
    assert_eq!(
        a.view()[0]["waiting"],
        "waiting: GitHub (GitHub /repos/o/r/actions/workflows/ci.yml/runs: 503)"
    );
    a.found(0, &runs(&format!("!7 {A} (7-0123456789ab-1)")), 4090);
    assert_eq!(a.dispatches[0].failures, 0);
    assert_eq!(a.view()[0]["waiting"], Value::Null);

    let mut refused = Actions::default();
    refused.queue(7, A, "fmt");
    assert_eq!(refused.due(Mirror::Passed, 1), [Due::Dispatch(0)]);
    refused.failed(
        0,
        "GitHub …/dispatches: 403 Resource not accessible by integration",
        1,
    );
    assert_eq!(
        refused.dispatches[0].dispatched_at, None,
        "dispatched again once due"
    );
    assert_eq!(refused.due(Mirror::Passed, 61), [Due::Dispatch(0)]);
}

#[test]
fn a_head_that_never_reached_github_is_pushed_again_with_backoff() {
    let mut a = Actions::default();
    a.queue(7, A, "fmt");
    a.queue(7, A, "test");
    assert_eq!(a.due(Mirror::Failed, 10), [Due::Mirror]);
    assert_eq!(a.due(Mirror::Failed, 69), []);
    assert_eq!(a.due(Mirror::Open, 70), []);
    assert_eq!(a.due(Mirror::Failed, 70), [Due::Mirror]);
    assert_eq!(a.dispatches[0].next_at, Some(370));
}

#[test]
fn an_old_heads_entries_are_dropped() {
    let mut a = dispatched();
    a.queue(7, B, "fmt");
    a.keep_head(Some(B));
    assert_eq!(a.dispatches.len(), 1);
    assert_eq!(a.dispatches[0].sha, B);
    assert!(
        !a.owns(A, "test"),
        "an old head's results are never applied"
    );
    a.keep_head(None);
    assert_eq!(a.dispatches.len(), 0);
}

#[test]
fn only_open_checks_are_dispatched_or_followed() {
    let mut a = dispatched();
    a.queue(7, A, "clippy");
    a.queue(7, A, "doc");
    a.keep_open(&["test".into(), "doc".into()]);
    assert_eq!(a.dispatches.len(), 2);
    assert_eq!(a.dispatches[1].checks, ["doc"]);
    assert!(
        !a.owns(A, "fmt") && a.owns(A, "test"),
        "a cancelled check is no longer followed"
    );
    a.keep_open(&[]);
    assert_eq!(
        a.dispatches.len(),
        1,
        "an undispatched entry with nothing open goes"
    );
    assert_eq!(
        a.due(Mirror::Failed, 10_000),
        [],
        "nothing is due for a change that stopped"
    );
}

#[test]
fn a_head_the_owner_moved_to_cloudflare_stays_there_until_a_new_head() {
    let mut a = dispatched();
    a.cloudflare = Some(A.into());
    assert!(a.on_cloudflare(A) && !a.on_cloudflare(B));
    a.keep_head(Some(A));
    assert!(a.on_cloudflare(A));
    a.keep_head(Some(B));
    assert!(
        !a.on_cloudflare(A) && !a.on_cloudflare(B),
        "a new head goes back to GitHub"
    );
}
