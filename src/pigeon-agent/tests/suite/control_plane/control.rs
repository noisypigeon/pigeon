//! The control plane's routing, push events and job reports; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1, §4.

use crate::harness::step;

use agent::control::{self, Outcome, Pipeline, Push, Report, Routed, Target};
use agent::core::change::{BOUND, Change, Check, Event};
use agent::wire::Request;
use serde_json::json;
use step::Apply;

const A: &str = "0123456789abcdef0123456789abcdef01234567";

fn request(op: &str, args: &serde_json::Value) -> Request {
    Request {
        operation: op.into(),
        args: args.as_object().cloned().unwrap_or_default(),
    }
}

#[test]
fn operations_route_to_events_with_the_access_identity() {
    let got = control::route(
        &request("change.approve", &json!({ "number": "7", "sha": A })),
        "owner@x",
        5,
    );
    assert_eq!(
        got,
        Ok(Routed::Event {
            number: 7,
            event: Event::Approve {
                by: "owner@x".into(),
                sha: A.into(),
                at: 5
            }
        })
    );
    assert_eq!(
        control::route(
            &request("change.authorize", &json!({ "number": 7 })),
            "owner@x",
            5
        ),
        Ok(Routed::Event {
            number: 7,
            event: Event::ApprovePlan {
                by: "owner@x".into(),
                at: 5
            }
        })
    );
    assert_eq!(
        control::route(&request("change.status", &json!({ "number": "0" })), "a", 1),
        Err("argument `number` must be a change's number".into())
    );
    let p = control::route(
        &request(
            "change.propose",
            &json!({ "slug": "worker-control", "title": "feat(x): y", "plan": "p", "scope": "a/**\n\n b/ \n" }),
        ),
        "agent",
        1,
    );
    assert_eq!(
        p,
        Ok(Routed::Propose {
            slug: "worker-control".into(),
            title: "feat(x): y".into(),
            plan: "p".into(),
            scope: vec!["a/**".into(), "b/".into()],
            mandate: None,
            session: None,
            affects: Vec::new(),
            full: false,
            resumes: None,
            after: None,
            asks: Vec::new(),
            brief: None,
        })
    );
    assert!(
        control::route(
            &request(
                "change.propose",
                &json!({ "slug": "Bad Slug", "title": "t", "plan": "p", "scope": "a" })
            ),
            "a",
            1
        )
        .is_err()
    );
    assert!(control::route(&request("infra.plan", &json!({})), "a", 1).is_err());
}

#[test]
fn a_push_to_a_change_branch_is_found_and_others_are_not() {
    let event = json!({ "type": "cf.artifacts.repo.pushed", "payload": { "repo": "pigeon", "ref": "refs/heads/change/9-worker-control", "after": A } });
    assert_eq!(
        control::push(&event),
        Some(Push {
            repo: Some("pigeon".into()),
            target: Target::Change(9),
            sha: A.into()
        })
    );
    assert_eq!(
        control::push(&json!({ "ref": "refs/heads/change/9-x", "after": A })).map(|p| p.target),
        Some(Target::Change(9)),
        "a payload without its envelope"
    );
    assert_eq!(
        control::push(&json!({ "ref": "refs/heads/main", "after": A })).map(|p| p.target),
        Some(Target::Main),
        "main goes to its pipeline"
    );
    for other in [
        json!({ "ref": "refs/heads/other", "after": A }),
        json!({ "ref": "refs/tags/v1", "after": A }),
        json!({ "ref": "refs/heads/change/9-x", "after": "0".repeat(40) }),
        json!({ "ref": "refs/heads/change/9-x", "after": "abc" }),
        json!({ "ref": "refs/heads/change/9-x" }),
    ] {
        assert_eq!(control::push(&other), None, "{other}");
    }
}

fn pushed() -> Change {
    let mut c = Change::propose(
        9,
        "worker-control",
        "feat(x): y",
        "the plan",
        vec!["a/**".into()],
        "agent",
    );
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("approves");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c
}

#[test]
fn a_job_spec_carries_the_approved_scope_and_never_shows_its_token() {
    let s = control::spec(
        &pushed(),
        BOUND,
        "https://git.example/pigeon.git",
        "tok-secret",
    )
    .expect("spec");
    assert_eq!(s.id, "9-0123456789ab-bound");
    assert_eq!(s.branch, "change/9-worker-control");
    assert_eq!(s.scope, ["a/**"]);
    assert_eq!(s.base, "main");
    assert!(!format!("{s:?}").contains("tok-secret"));
    assert!(!format!("{s:?}").contains("git.example"));
    assert!(control::spec(&pushed(), "fmt", "r", "t").is_err());
    assert_eq!(
        control::job_id(3, A, "clippy pigeon --features infra"),
        "3-0123456789ab-clippy-pigeon---features-infra"
    );
}

#[test]
fn reports_become_events() {
    let report = |outcome, violations: Vec<&str>| Report {
        id: "x".into(),
        outcome,
        log: String::new(),
        violations: violations.into_iter().map(Into::into).collect(),
        full: vec![],
        gate: None,
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec!["cargo".into()],
        }],
        merged: None,
        published: vec![],
        ios: false,
        workers: vec![],
        passed: vec![],
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
        evictions: vec![],
        forecast: vec![],
        built_on: None,
        ledger: None,
        pool: None,
        serving: None,
        pipeline: false,
        standby: None,
        patch: None,
        own: None,
    };
    assert_eq!(
        control::event_of("fmt", A, &report(Outcome::Running, vec![])),
        None
    );
    assert_eq!(
        control::event_of("fmt", A, &report(Outcome::Failed, vec![])),
        Some(Event::Finished {
            job: "fmt".into(),
            sha: A.into(),
            ok: false
        })
    );
    let Some(Event::Bounded {
        violations, checks, ..
    }) = control::event_of(BOUND, A, &report(Outcome::Failed, vec![]))
    else {
        panic!("bounded");
    };
    assert_eq!(
        violations,
        ["the bound job failed; see its log"],
        "a failed bound job always refuses"
    );
    assert_eq!(checks.len(), 1);
    let Some(Event::Bounded { violations, .. }) =
        control::event_of(BOUND, A, &report(Outcome::Passed, vec![]))
    else {
        panic!("bounded");
    };
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn a_log_keeps_its_tail_on_a_character_boundary() {
    let long = format!("{}é{}", "x".repeat(10), "y".repeat(control::LOG_TAIL - 1));
    let t = control::tail(&long);
    assert!(t.len() <= control::LOG_TAIL);
    assert!(t.ends_with('y'));
    assert_eq!(control::tail("short"), "short");
    assert_eq!(control::last("aé", 1), "", "never half a character");
    assert_eq!(control::last("aé", 2), "é");
}

#[test]
fn events_round_trip_their_wire_form() {
    let e = Event::Bounded {
        sha: A.into(),
        violations: vec![],
        checks: vec![],
        full: vec![],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    };
    let text = serde_json::to_string(&e).expect("serialize");
    assert!(text.contains("\"type\":\"bounded\""), "{text}");
    assert_eq!(
        serde_json::from_str::<Event>(&text).expect("deserialize"),
        e
    );
}

const SCHEMA: [&str; 3] = [
    include_str!("../../../migrations/0001_schema_v0.sql"),
    include_str!("../../../migrations/0002_changes.sql"),
    include_str!("../../../migrations/0003_change_sessions.sql"),
];

fn db() -> rusqlite::Connection {
    let db = rusqlite::Connection::open_in_memory().expect("sqlite");
    db.execute_batch("PRAGMA foreign_keys = ON;").expect("fk");
    for s in SCHEMA {
        db.execute_batch(s).expect("migrates");
    }
    db
}

fn run(db: &rusqlite::Connection, s: &control::Statement) {
    let params: Vec<rusqlite::types::Value> = s
        .params
        .iter()
        .map(|v| match v {
            serde_json::Value::Null => rusqlite::types::Value::Null,
            serde_json::Value::Number(n) => {
                rusqlite::types::Value::Integer(n.as_i64().expect("int"))
            }
            serde_json::Value::String(t) => rusqlite::types::Value::Text(t.clone()),
            other => panic!("{other}"),
        })
        .collect();
    db.execute(s.sql, rusqlite::params_from_iter(params))
        .unwrap_or_else(|e| panic!("{}: {e}", s.sql));
}

fn number(db: &rusqlite::Connection, s: &control::Statement) -> i64 {
    let params: Vec<rusqlite::types::Value> = s
        .params
        .iter()
        .map(|v| match v {
            serde_json::Value::Number(n) => {
                rusqlite::types::Value::Integer(n.as_i64().expect("int"))
            }
            serde_json::Value::String(t) => rusqlite::types::Value::Text(t.clone()),
            _ => rusqlite::types::Value::Null,
        })
        .collect();
    db.query_row(s.sql, rusqlite::params_from_iter(params), |r| r.get(0))
        .expect("row")
}

#[test]
fn the_mirror_writes_every_state_the_migration_allows() {
    let db = db();
    let logs = serde_json::Map::new();
    assert_eq!(
        number(&db, &control::next_number("pigeon", "pigeon", 10)),
        10,
        "the floor"
    );
    let mut c = Change::propose(10, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    for s in control::mirror("pigeon", "pigeon", &c, &logs, 1) {
        run(&db, &s);
    }
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 2,
    })
    .expect("approves");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Bounded {
        sha: A.into(),
        violations: vec![],
        full: vec![],
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    })
    .expect("bounded");
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: A.into(),
    })
    .expect("starts");
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: A.into(),
        ok: true,
    })
    .expect("passes");
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 3,
    })
    .expect("approves");
    let mut logs = serde_json::Map::new();
    logs.insert("fmt".into(), "ci/10-0123456789ab-fmt.log".into());
    for s in control::mirror("pigeon", "pigeon", &c, &logs, 3) {
        run(&db, &s);
    }
    let state: String = db
        .query_row("SELECT state FROM changes WHERE number = 10", [], |r| {
            r.get(0)
        })
        .expect("row");
    assert_eq!(state, "approved");
    let jobs: i64 = db
        .query_row(
            "SELECT count(*) FROM jobs WHERE change = 10 AND log IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(jobs, 1);
    assert_eq!(
        number(&db, &control::next_number("pigeon", "pigeon", 1)),
        11
    );
    run(
        &db,
        &control::audit(4, "owner", "change.approve", "pigeon", Some("10"), "ok"),
    );
    let listed: i64 = db
        .query_row(
            "SELECT count(*) FROM changes WHERE state = 'approved'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(listed, 1);
    let s = control::list("pigeon", "pigeon", Some("approved"));
    let params: Vec<rusqlite::types::Value> = s
        .params
        .iter()
        .map(|v| match v {
            serde_json::Value::String(t) => rusqlite::types::Value::Text(t.clone()),
            _ => rusqlite::types::Value::Null,
        })
        .collect();
    let n: i64 = db
        .query_row(s.sql, rusqlite::params_from_iter(params), |r| r.get(0))
        .expect("row");
    assert_eq!(n, 10);
}

const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

fn deploy_report(outcome: Outcome) -> Report {
    Report {
        id: "x".into(),
        outcome,
        log: String::new(),
        violations: vec![],
        full: vec![],
        gate: None,
        checks: vec![],
        merged: None,
        published: vec!["sites/docs".into()],
        ios: false,
        workers: vec!["pigeon-control: v1 → v2".into()],
        passed: vec![],
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
        evictions: vec![],
        forecast: vec![],
        built_on: None,
        ledger: None,
        pool: None,
        serving: None,
        pipeline: false,
        standby: None,
        patch: None,
        own: None,
    }
}

#[test]
fn main_deploys_one_at_a_time_and_always_the_newest_push() {
    let mut p = Pipeline::default();
    p.pushed(A);
    assert_eq!(p.start().as_deref(), Some(A));
    p.pushed(B);
    p.pushed(C);
    assert_eq!(p.start(), None, "one deploy at a time");
    p.finished(&deploy_report(Outcome::Passed), 10);
    assert_eq!(p.deployed.as_deref(), Some(A));
    assert_eq!(
        p.start().as_deref(),
        Some(C),
        "B is skipped; C publishes what changed since A"
    );
    p.finished(&deploy_report(Outcome::Failed), 11);
    assert_eq!(
        p.deployed.as_deref(),
        Some(A),
        "a failed deploy isn't the last deployed"
    );
    assert_eq!(p.start(), None, "nothing new to deploy until the next push");
    assert_eq!(p.history.len(), 2);
    assert_eq!(p.history[0].sha, C);
    assert_eq!(p.history[0].outcome, Outcome::Failed);
    assert_eq!(p.target, None, "a finished deploy clears its target");
    p.pushed("not a sha");
    assert_eq!(p.target, None, "only a SHA is a push");
}

#[test]
fn a_merge_is_carried_by_the_deploys_of_it_and_later_pushes() {
    use agent::core::change::stage::Deploy;
    let mut p = Pipeline::default();
    assert_eq!(p.carrying(A), Deploy::Unknown, "a push not kept");
    p.pushed(A);
    assert_eq!(p.carrying(A), Deploy::Pending);
    p.start();
    p.pushed(B);
    p.pushed(C);
    assert_eq!(p.carrying(A), Deploy::Running);
    assert_eq!(p.carrying(B), Deploy::Pending, "A's deploy doesn't carry B");
    p.finished(&deploy_report(Outcome::Passed), 10);
    assert_eq!(p.carrying(A), Deploy::Passed);
    p.start();
    assert_eq!(p.carrying(B), Deploy::Running, "C's deploy carries B");
    p.finished(&deploy_report(Outcome::Failed), 11);
    assert!(matches!(p.carrying(B), Deploy::Failed(_)));
    assert_eq!(
        p.carrying(A),
        Deploy::Passed,
        "a later failure doesn't undo A"
    );
    p.pushed(C);
    assert_eq!(p.pushes.len(), 3, "the same push twice is kept once");
    for i in 0..control::PUSHES {
        p.pushed(&format!("{i:040x}"));
    }
    assert_eq!(p.pushes.len(), control::PUSHES);
    assert_eq!(
        p.carrying(A),
        Deploy::Unknown,
        "the oldest pushes are dropped"
    );
}

#[test]
fn the_newest_build_a_deploy_stored_is_what_jobs_run() {
    let build = |sha: &str| agent::control::Build {
        sha: sha.into(),
        sha256: "ab".repeat(32),
        judge_sha256: None,
        judge_sha: None,
    };
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    p.finished(
        &Report {
            build: Some(build(A)),
            ..deploy_report(Outcome::Failed)
        },
        10,
    );
    assert_eq!(
        p.latest_build,
        Some(build(A)),
        "a build from main is kept even when its deploy failed"
    );
    p.pushed(B);
    p.start();
    p.finished(&deploy_report(Outcome::Passed), 11);
    assert_eq!(
        p.latest_build,
        Some(build(A)),
        "a deploy that built nothing keeps the last build"
    );
    p.pushed(C);
    p.start();
    p.finished(
        &Report {
            build: Some(build(B)),
            ..deploy_report(Outcome::Passed)
        },
        12,
    );
    assert_eq!(
        p.latest_build,
        Some(build(A)),
        "a build of another commit than the deploy's is refused"
    );
    let shown = serde_json::to_value(&p).expect("json");
    assert_eq!(shown["latest_build"]["sha"], A);
    let old: Pipeline = serde_json::from_value(serde_json::json!({ "history": [] })).expect("old");
    assert_eq!(
        old.latest_build, None,
        "a pipeline stored before builds still reads"
    );
}

#[test]
fn the_newest_build_keeps_each_binarys_own_commit() {
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    p.finished(
        &Report {
            build: Some(agent::control::Build {
                sha: A.into(),
                sha256: "a".repeat(64),
                judge_sha256: Some("b".repeat(64)),
                judge_sha: None,
            }),
            ..deploy_report(Outcome::Passed)
        },
        10,
    );
    let l = p.latest_build.clone().expect("build");
    assert_eq!(l.sha, A);
    assert_eq!(l.judge_sha, None);
    p.pushed(B);
    p.start();
    p.finished(
        &Report {
            build: Some(agent::control::Build {
                sha: B.into(),
                sha256: "c".repeat(64),
                judge_sha256: None,
                judge_sha: None,
            }),
            ..deploy_report(Outcome::Passed)
        },
        11,
    );
    let l = p.latest_build.clone().expect("build");
    assert_eq!(l.sha, B);
    assert_eq!(l.sha256, "c".repeat(64));
    assert_eq!(l.judge_sha.as_deref(), Some(A));
    assert_eq!(l.judge_sha256, Some("b".repeat(64)));
    p.pushed(C);
    p.start();
    p.finished(
        &Report {
            build: Some(agent::control::Build {
                sha: C.into(),
                sha256: String::new(),
                judge_sha256: Some("d".repeat(64)),
                judge_sha: None,
            }),
            ..deploy_report(Outcome::Passed)
        },
        12,
    );
    let l = p.latest_build.clone().expect("build");
    assert_eq!(l.sha, B);
    assert_eq!(l.judge_sha.as_deref(), Some(C));
    assert_eq!(l.judge_sha256, Some("d".repeat(64)));
}

#[test]
fn a_lost_deploy_retries_then_fails() {
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    for _ in 1..agent::core::platform::runs::MAX_ATTEMPTS {
        assert_eq!(p.lost(1).as_deref(), Some(A));
    }
    assert_eq!(p.lost(1), None);
    assert_eq!(p.history[0].outcome, Outcome::Failed);
    assert!(p.running.is_none());
}

#[test]
fn a_deploy_main_moved_past_is_superseded_and_the_newest_push_goes_next() {
    use agent::core::change::stage::Deploy;
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    p.finished(&deploy_report(Outcome::Passed), 1);
    p.pushed(B);
    assert_eq!(p.start().as_deref(), Some(B));
    p.pushed(C);
    let mut r = deploy_report(Outcome::Superseded);
    r.violations = vec![format!("main moved on to {C}")];
    p.finished(&r, 2);
    assert_eq!(p.deployed.as_deref(), Some(A), "superseded isn't deployed");
    assert_eq!(p.history[0].outcome, Outcome::Superseded);
    assert_eq!(
        p.history[0].violations,
        vec![format!("main moved on to {C}")]
    );
    assert_eq!(
        p.carrying(B),
        Deploy::Pending,
        "C's deploy carries B; a superseded deploy is no failure"
    );
    assert_eq!(p.start().as_deref(), Some(C), "the newest push goes next");
}

#[test]
fn a_lost_deploy_main_moved_past_is_superseded_not_retried() {
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    p.pushed(B);
    assert_eq!(
        p.lost(1).as_deref(),
        Some(B),
        "the newest push starts at once"
    );
    assert_eq!(p.history[0].sha, A);
    assert_eq!(p.history[0].outcome, Outcome::Superseded);
    assert_eq!(p.running, Some((B.to_string(), 1)));
}

#[test]
fn a_deploy_spec_never_shows_its_credentials() {
    let env = std::collections::BTreeMap::from([(
        "CLOUDFLARE_API_TOKEN".to_string(),
        "deploy-secret-value".to_string(),
    )]);
    let s = control::deploy_spec(
        "https://git.example/p.git",
        "read-token-value",
        A,
        Some(B),
        env,
    );
    assert_eq!(s.id, control::deploy_id(A));
    assert_eq!(s.branch, "main");
    let shown = format!("{s:?}");
    assert!(
        !shown.contains("deploy-secret-value") && !shown.contains("read-token-value"),
        "{shown}"
    );
}

#[test]
fn a_mirror_runs_on_runner_and_never_shows_its_tokens() {
    let s = control::mirror_spec(
        "https://git.example/p.git",
        "read-token-value",
        A,
        "https://github.com/o/p",
        "ghs-mirror-token",
    );
    assert_eq!(s.id, control::mirror_id(A));
    assert_eq!(s.job, control::MIRROR);
    assert_eq!(s.branch, "main");
    assert_eq!(
        s.env.get(control::MIRROR_TO).map(String::as_str),
        Some("https://github.com/o/p")
    );
    let shown = format!("{s:?}");
    assert!(
        !shown.contains("ghs-mirror-token") && !shown.contains("read-token-value"),
        "{shown}"
    );
    assert_eq!(
        control::class_of(control::MIRROR),
        agent::control::Class::Runner
    );
}

#[test]
fn a_change_is_unmirrored_by_its_branch_on_a_runner() {
    let s = control::unmirror_spec(&pushed(), "https://github.com/o/r.git", "ghs-unmirror");
    assert_eq!(s.id, "9-0123456789ab-unmirror");
    assert_eq!(s.job, control::UNMIRROR);
    assert_eq!(s.branch, "change/9-worker-control");
    assert!(s.token.is_empty(), "it reads nothing from Artifacts");
    assert_eq!(s.env[control::MIRROR_TOKEN], "ghs-unmirror");
    assert!(!format!("{s:?}").contains("ghs-unmirror"));
    assert_eq!(
        control::class_of(control::UNMIRROR),
        agent::control::Class::Runner
    );
    let refused = Event::Bounded {
        sha: A.into(),
        violations: vec!["no".into()],
        checks: vec![],
        full: vec![],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    };
    assert_eq!(control::with_mirror(refused.clone()), refused);
    let other = Event::Close;
    assert_eq!(control::with_mirror(other.clone()), other);
}

#[test]
fn no_free_container_is_waiting_not_a_lost_job() {
    assert!(control::no_capacity(
        "Maximum number of running container instances exceeded. Try again later"
    ));
    assert!(control::no_capacity(
        "the Container did not answer: Error: There is no container instance that can be provided to this Durable Object, try again later"
    ));
    assert!(!control::no_capacity(
        "the Container did not answer: connection refused"
    ));
}

#[test]
fn checks_and_deploys_run_on_checker_and_git_jobs_on_runner() {
    use agent::control::Class;
    use agent::core::change::MERGE;
    assert_eq!(control::class_of(BOUND), Class::Runner);
    assert_eq!(control::class_of(MERGE), Class::Runner);
    assert_eq!(
        control::class_of(control::DEPLOY),
        Class::Checker,
        "a deploy builds the Worker"
    );
    assert_eq!(control::class_of("fmt"), Class::Checker);
    assert_eq!(
        control::class_of("clippy agent worker (wasm)"),
        Class::Checker
    );
    assert_eq!(Class::Checker.binding(), "CHECKER");
    assert_eq!(Class::Runner.binding(), "RUNNER");
}

#[test]
fn a_change_runs_at_most_six_jobs_at_once() {
    assert!(control::may_start(0));
    assert!(control::may_start(control::MAX_RUNNING - 1));
    assert!(!control::may_start(control::MAX_RUNNING));
    assert_eq!(control::MAX_RUNNING, 6);
}

#[test]
fn git_tokens_show_metadata_and_never_a_value() {
    assert_eq!(
        control::route(&request("git.tokens", &json!({})), "agent", 1),
        Ok(Routed::GitTokens)
    );
    let api = json!([
        { "id": "t1", "scope": "write", "state": "expired", "created_at": "a", "expires_at": "b",
          "token": "art_v1_x", "plaintext": "art_v1_y", "value": "z", "secret": "s" },
        { "id": "t2", "scope": "read", "state": "active" }
    ]);
    let rows = control::token_rows(&api);
    assert_eq!(
        rows,
        json!([
            { "id": "t1", "scope": "write", "state": "expired", "created_at": "a", "expires_at": "b" },
            { "id": "t2", "scope": "read", "state": "active" }
        ])
    );
    assert!(!rows.to_string().contains("art_v1"));
    assert_eq!(
        control::token_rows(&json!({ "tokens": [{ "id": "t3", "token": "v" }] })),
        json!([{ "id": "t3" }])
    );
    assert_eq!(control::token_rows(&json!(null)), json!([]));
}

#[test]
fn a_deploy_is_retried_on_request_and_only_then() {
    let mut p = Pipeline::default();
    assert_eq!(p.retry(None), None, "nothing deployed or failed yet");
    p.pushed(A);
    assert_eq!(p.start().as_deref(), Some(A));
    assert_eq!(p.retry(None), None, "a deploy runs");
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(
        p.history[0].workers,
        vec!["pigeon-control: v1 → v2".to_string()]
    );
    assert_eq!(
        p.retry(None).as_deref(),
        Some(A),
        "a passed push is deployed again"
    );
    assert_eq!(p.deployed, None, "from nothing: everything is deployed");
    assert_eq!(p.retry(None), None, "a deploy runs");
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(p.deployed.as_deref(), Some(A));
    p.pushed(B);
    assert_eq!(p.start().as_deref(), Some(B));
    p.finished(&deploy_report(Outcome::Failed), 2);
    assert_eq!(p.start(), None, "a failure waits");
    assert_eq!(p.retry(None).as_deref(), Some(B), "until it is retried");
    assert_eq!(p.retry(None), None, "once at a time");
    p.finished(&deploy_report(Outcome::Passed), 3);
    assert_eq!(p.deployed.as_deref(), Some(B));
}

#[test]
fn deploys_reference_the_newest_images_and_the_owners_push_wins() {
    let imgs = |source: &str| agent::control::Images {
        source: source.into(),
        images: [(
            "pigeon-control/Checker".to_string(),
            format!("registry/x:{source}"),
        )]
        .into_iter()
        .collect(),
    };
    let mut p = Pipeline::default();
    assert_eq!(p.images_for_deploy(None), None, "nothing yet");
    assert_eq!(
        p.images_for_deploy(Some(imgs(A))),
        Some(imgs(A)),
        "the secret seeds the pipeline"
    );
    p.pushed(B);
    p.start();
    p.finished(
        &Report {
            images: Some(imgs(B)),
            ..deploy_report(Outcome::Failed)
        },
        1,
    );
    assert_eq!(
        p.images_for_deploy(Some(imgs(A))),
        Some(imgs(B)),
        "images a deploy built win over an unchanged secret"
    );
    p.pushed(C);
    p.start();
    p.finished(
        &Report {
            images: Some(imgs(A)),
            ..deploy_report(Outcome::Passed)
        },
        2,
    );
    assert_eq!(
        p.images_for_deploy(Some(imgs(A))),
        Some(imgs(B)),
        "images of another commit than the deploy's are refused"
    );
    assert_eq!(
        p.images_for_deploy(Some(imgs(C))),
        Some(imgs(C)),
        "a changed secret is the owner's push, and wins"
    );
    let shown = serde_json::to_string(&p.shown()).expect("json");
    assert!(
        !shown.contains("registry/x"),
        "references stay hidden: {shown}"
    );
    assert!(shown.contains(C), "{shown}");
}

#[test]
fn a_waived_job_mirrors_with_no_row() {
    let db = db();
    let logs = serde_json::Map::new();
    let mut c = Change::propose(14, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 2,
    })
    .expect("approves");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Bounded {
        sha: A.into(),
        violations: vec![],
        full: vec![],
        checks: vec![
            Check {
                name: "fmt".into(),
                argv: vec![],
            },
            Check {
                name: "clippy".into(),
                argv: vec![],
            },
        ],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    })
    .expect("bounded");
    for s in control::mirror("pigeon", "pigeon", &c, &logs, 2) {
        run(&db, &s);
    }
    c.apply(Event::Waived {
        job: "fmt".into(),
        sha: A.into(),
        lane: 1,
    })
    .expect("waives");
    for s in control::mirror("pigeon", "pigeon", &c, &logs, 3) {
        run(&db, &s);
    }
    let statuses: Vec<(String, String)> = db
        .prepare("SELECT name, status FROM jobs WHERE change = 14 AND name IN ('fmt', 'clippy') ORDER BY name")
        .expect("prepares")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("queries")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("clippy".to_string(), "queued".to_string()),
            ("fmt".to_string(), "queued".to_string()),
        ],
        "a waived job keeps its last row and is never written as waived or passed"
    );
}

#[test]
fn the_mirror_keeps_a_changes_session_and_affects_for_overlaps() {
    let db = db();
    let logs = serde_json::Map::new();
    let mut c = Change::propose(12, "x", "docs(adr): x", "p", vec!["a.md".into()], "agent");
    c.session = Some("s-a".into());
    c.affects = vec!["src/**".into()];
    for s in control::mirror("pigeon", "pigeon", &c, &logs, 1) {
        run(&db, &s);
    }
    let old = Change::propose(13, "y", "feat(y): y", "p", vec!["b/**".into()], "agent");
    for s in control::mirror("pigeon", "pigeon", &old, &logs, 1) {
        run(&db, &s);
    }
    let q = control::open("pigeon", "pigeon");
    let mut stmt = db.prepare(q.sql).expect("prepares");
    let rows: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params!["pigeon", "pigeon"], |r| {
            Ok(serde_json::json!({
                "number": r.get::<_, i64>(0)?,
                "state": r.get::<_, String>(1)?,
                "session": r.get::<_, Option<String>>(2)?,
                "scope": r.get::<_, String>(3)?,
                "affects": r.get::<_, String>(4)?,
            }))
        })
        .expect("queries")
        .map(|r| r.expect("row"))
        .collect();
    let open = control::open_rows(&rows);
    assert_eq!(open.len(), 2);
    assert_eq!(open[0].session.as_deref(), Some("s-a"));
    assert_eq!(open[0].affects, ["src/**"]);
    assert_eq!(open[1].session, None, "a change with no session");
    assert_eq!(open[1].affects, Vec::<String>::new());
}

#[test]
fn a_merge_commit_names_the_approvals_kind_never_an_identity() {
    for (by, shown) in [
        ("owner@example.com", "owner"),
        ("a07759a856eb15d9711537fe6bec2108.access", "owner"),
        ("grant:owner@example.com", "grant:owner"),
        ("grant:4f2a", "grant:owner"),
        ("policy:routine", "policy:routine"),
        ("mandate:7", "mandate:7"),
        ("", "owner"),
    ] {
        assert_eq!(control::public_approver(by), shown, "{by}");
    }
}

#[test]
fn code_checks_go_to_github_only_on_a_mirrored_change() {
    use agent::actions::Mirror;
    let mut c = pushed();
    c.apply(control::with_mirror(Event::Bounded {
        sha: A.into(),
        violations: vec![],
        full: vec![],
        checks: vec![
            Check {
                name: "fmt".into(),
                argv: vec!["cargo".into(), "fmt".into()],
            },
            Check {
                name: "policy.check".into(),
                argv: vec!["cargo".into(), "run".into()],
            },
        ],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    }))
    .expect("bounded");
    assert!(control::to_github(&c, "fmt", true, false));
    assert!(
        !control::to_github(&c, "fmt", false, false),
        "a Worker that doesn't mirror"
    );
    assert!(!control::to_github(&c, "policy.check", true, false));
    assert!(!control::to_github(&c, control::MIRROR, true, false));
    assert!(
        !control::to_github(&c, "fmt", true, true),
        "the owner's CI window keeps code checks on the Checker"
    );
    assert_eq!(control::mirror_state(&c), Mirror::Open);
    let names: Vec<String> = control::checks_of(&c).into_iter().map(|k| k.name).collect();
    assert_eq!(names, ["bound", "fmt", "policy.check", "mirror"]);

    let mut plain = pushed();
    plain
        .apply(Event::Bounded {
            sha: A.into(),
            violations: vec![],
            full: vec![],
            checks: vec![Check {
                name: "fmt".into(),
                argv: vec!["cargo".into(), "fmt".into()],
            }],
            tiers: None,
            gate: None,
            patch: None,
            own: None,
        })
        .expect("bounded");
    assert!(
        !control::to_github(&plain, "fmt", true, false),
        "a change bounded before the mirror job keeps its Checker"
    );
    assert_eq!(control::mirror_state(&plain), Mirror::Failed);
}

fn job(name: &str, cmd: &str, status: agent::core::change::JobStatus) -> agent::core::change::Job {
    agent::core::change::Job {
        name: name.into(),
        argv: cmd.split(' ').map(str::to_string).collect(),
        status,
        attempts: 1,
        start_failures: 0,
        last_error: None,
        lane: None,
    }
}

#[test]
fn ci_rerun_on_cloudflare_is_the_owners_break_glass() {
    let req = |on: Option<&str>| {
        let mut args = json!({ "number": 9, "job": "fmt", "ticket": "t" });
        if let Some(o) = on {
            args["on"] = json!(o);
        }
        request("ci.rerun", &args)
    };
    let r = control::retarget(&req(Some("cloudflare")));
    assert_eq!(r.operation, control::BREAK_GLASS);
    let mut keys: Vec<&String> = r.args.keys().collect();
    keys.sort();
    assert_eq!(keys, ["number", "ticket"]);
    for kept in [req(Some("github")), req(None)] {
        let r = control::retarget(&kept);
        assert_eq!(
            (r.operation.as_str(), r.args.clone()),
            ("ci.rerun", kept.args)
        );
    }
    assert!(matches!(
        control::route(&req(Some("github")), "a", 1),
        Ok(Routed::Event {
            event: Event::Rerun { .. },
            ..
        })
    ));
    assert!(control::route(&req(Some("cloudflare")), "a", 1).is_err());
    assert!(control::route(&req(Some("elsewhere")), "a", 1).is_err());
    assert!(matches!(
        control::route(&r, "owner", 1),
        Ok(Routed::BreakGlass { number: 9 })
    ));
}

#[test]
fn the_break_glass_moves_a_heads_unpassed_code_checks_to_the_checker() {
    use agent::actions::Actions;
    use agent::core::change::{JobStatus, State};
    let mut c = pushed();
    c.state = State::Bounded;
    c.jobs = vec![
        job("bound", "", JobStatus::Passed),
        job("fmt", "cargo fmt --all", JobStatus::Passed),
        job("test", "cargo test --workspace", JobStatus::Running),
        job("clippy", "cargo clippy --all-targets", JobStatus::Queued),
        job(
            "policy.check",
            "cargo run -p pigeon-harness",
            JobStatus::Running,
        ),
        job(control::MIRROR, "", JobStatus::Passed),
    ];
    let mut github = Actions::default();
    github.queue(9, A, "test");
    let (events, start) = control::break_glass(&c, &mut github).expect("applies");
    assert_eq!(
        events,
        [Event::Lost {
            job: "test".into(),
            sha: A.into()
        }]
    );
    assert_eq!(start, ["clippy"]);
    assert!(github.on_cloudflare(A));
    assert_eq!(github.dispatches.len(), 0, "nothing more is dispatched");

    let mut failed = pushed();
    failed.state = State::Failed;
    failed.jobs = vec![
        job("bound", "", JobStatus::Passed),
        job("test", "cargo test --workspace", JobStatus::Failed),
        job("fmt", "cargo fmt --all", JobStatus::Cancelled),
    ];
    let (events, start) = control::break_glass(&failed, &mut Actions::default()).expect("applies");
    assert_eq!(events, [Event::Rerun { job: "test".into() }]);
    assert_eq!(
        start.len(),
        0,
        "the re-run queues those its failure cancelled"
    );
    let mut after = failed.clone();
    let effects = after.apply(events[0].clone()).expect("reruns");
    assert_eq!(effects.len(), 2, "test and fmt start again: {effects:?}");

    let mut done = failed;
    done.jobs.retain(|j| j.name == "bound");
    assert!(
        control::break_glass(&done, &mut Actions::default()).is_err(),
        "nothing to run"
    );
}

#[test]
fn opening_the_ci_window_moves_a_change_waiting_on_github() {
    use agent::actions::Actions;
    use agent::core::change::{JobStatus, State};
    let mut c = pushed();
    c.state = State::Bounded;
    c.jobs = vec![
        job("bound", "", JobStatus::Passed),
        job("fmt", "cargo fmt --all", JobStatus::Passed),
        job("test", "cargo test --workspace", JobStatus::Running),
        job(control::MIRROR, "", JobStatus::Passed),
    ];
    assert!(control::window_moves(&c));
    let mut github = Actions::default();
    github.queue(9, A, "test");
    control::break_glass(&c, &mut github).expect("applies");
    assert!(github.on_cloudflare(A));
    assert_eq!(github.dispatches.len(), 0, "its GitHub dispatch is dropped");

    let mut failed = c.clone();
    failed.jobs[2].status = JobStatus::Failed;
    assert!(
        !control::window_moves(&failed),
        "a failed check waits for ci_rerun"
    );
    let mut closed = c.clone();
    closed.state = State::Closed;
    assert!(!control::window_moves(&closed));
    let mut passed = c;
    passed.jobs[2].status = JobStatus::Passed;
    assert!(!control::window_moves(&passed));
}

#[test]
fn ci_window_routes_with_its_hours() {
    let ask = |h: serde_json::Value| {
        control::route(&request("ci.window", &json!({ "hours": h })), "a", 1)
    };
    assert_eq!(ask(json!("6")), Ok(Routed::CiWindow { hours: 6 }));
    assert_eq!(ask(json!(0)), Ok(Routed::CiWindow { hours: 0 }));
    assert!(ask(json!(25)).is_err());
    assert!(ask(json!("six")).is_err());
}

#[test]
fn a_full_merge_gate_can_be_asked_for_at_proposal_and_at_merge() {
    let propose = control::route(
        &request(
            "change.propose",
            &json!({ "slug": "x", "title": "feat(x): y", "plan": "p", "scope": "a/**", "full": true }),
        ),
        "agent",
        1,
    );
    assert!(matches!(propose, Ok(Routed::Propose { full: true, .. })));
    let merge =
        |args: serde_json::Value| control::route(&request("change.merge", &args), "owner", 1);
    assert!(matches!(
        merge(json!({ "number": "7", "full": "true" })),
        Ok(Routed::Event {
            event: Event::MergeFull { .. },
            ..
        })
    ));
    assert!(matches!(
        merge(json!({ "number": "7" })),
        Ok(Routed::Event {
            event: Event::Merge { .. },
            ..
        })
    ));
}

#[test]
fn an_eviction_job_runs_on_mains_head_with_its_candidates_and_one_id_an_hour() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let candidates = serde_json::json!([{ "number": 79, "fields": { "state": "pushed" } }]);
    let spec = control::eviction_spec("https://r", "t", sha, &candidates, 492_000);
    assert_eq!(spec.id, "main-0123456789ab-eviction-492000");
    assert_eq!((spec.branch.as_str(), spec.sha.as_str()), ("main", sha));
    assert_eq!(spec.job, control::EVICTION);
    assert_eq!(control::class_of(&spec.job), control::Class::Runner);
    let held: serde_json::Value =
        serde_json::from_str(&spec.env[control::CANDIDATES]).expect("JSON");
    assert_eq!(held, candidates);
    assert_eq!(spec.env.len(), 1, "no credential rides with it");
}

#[test]
fn a_failed_deploy_always_says_why_and_the_deployed_head_never_starts_again() {
    use agent::control::{Outcome, Pipeline, Report, failure};
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let mut p = Pipeline::default();
    p.pushed(sha);
    assert_eq!(p.start().as_deref(), Some(sha));
    p.finished(
        &Report::of(
            "d",
            Outcome::Failed,
            "built\nwrangler: 10021 the script is too large\n\n",
        ),
        1,
    );
    assert_eq!(
        p.history[0].violations,
        ["wrangler: 10021 the script is too large"],
        "the log's last line"
    );
    let mut said = Report::of("d", Outcome::Failed, "noise");
    said.violations = vec!["the image is missing".into()];
    assert_eq!(failure(&said), "the image is missing", "a violation first");
    assert_eq!(
        failure(&Report::of("d", Outcome::Failed, "")),
        "the deploy failed with no output"
    );
    // Passed, the head is deployed, and a push of it starts nothing.
    p.pushed(sha);
    p.start();
    p.finished(&Report::of("d", Outcome::Passed, ""), 2);
    p.pushed(sha);
    assert_eq!(p.start(), None);
}

#[test]
fn a_pin_and_a_resumed_proposal_are_routed() {
    let req = |op: &str, args: serde_json::Value| agent::wire::Request {
        operation: op.into(),
        args: args.as_object().cloned().unwrap_or_default(),
    };
    let pin = control::route(
        &req(
            "change.pin",
            serde_json::json!({ "number": 79, "off": "true" }),
        ),
        "owner",
        1,
    );
    assert!(
        matches!(
            pin,
            Ok(Routed::Pin {
                number: 79,
                off: true
            })
        ),
        "{pin:?}"
    );
    let front = control::route(
        &req("change.front", serde_json::json!({ "number": 585 })),
        "owner",
        1,
    );
    assert!(
        matches!(&front, Ok(Routed::Front { number: 585, by }) if by == "owner"),
        "{front:?}"
    );
    let propose = serde_json::json!({
        "slug": "x", "title": "feat(x): y", "plan": "p", "scope": "a/**", "resumes": "79",
    });
    match control::route(&req("change.propose", propose.clone()), "agent", 1) {
        Ok(Routed::Propose { resumes, .. }) => assert_eq!(resumes, Some(79)),
        other => panic!("{other:?}"),
    }
    let mut bad = propose.clone();
    bad["resumes"] = serde_json::json!("seventy");
    assert!(control::route(&req("change.propose", bad), "agent", 1).is_err());
    let mut after = propose;
    after["after"] = serde_json::json!("80");
    match control::route(&req("change.propose", after.clone()), "agent", 1) {
        Ok(Routed::Propose { after, .. }) => assert_eq!(after, Some(80)),
        other => panic!("{other:?}"),
    }
    after["after"] = serde_json::json!("eighty");
    assert!(control::route(&req("change.propose", after), "agent", 1).is_err());
}

#[test]
fn a_yield_names_the_change_it_waits_for_unless_it_ends() {
    let req = |args: serde_json::Value| agent::wire::Request {
        operation: "change.yield".into(),
        args: args.as_object().cloned().unwrap_or_default(),
    };
    let y = control::route(
        &req(serde_json::json!({ "number": 79, "to": "80", "reason": "same file" })),
        "s",
        1,
    );
    assert!(
        matches!(
            &y,
            Ok(Routed::Yield {
                number: 79,
                to: Some(80),
                off: false,
                ..
            })
        ),
        "{y:?}"
    );
    let off = control::route(
        &req(serde_json::json!({ "number": 79, "off": "true" })),
        "s",
        1,
    );
    assert!(
        matches!(
            off,
            Ok(Routed::Yield {
                to: None,
                off: true,
                ..
            })
        ),
        "{off:?}"
    );
    assert!(
        control::route(&req(serde_json::json!({ "number": 79 })), "s", 1).is_err(),
        "names `to`"
    );
}

#[test]
fn a_note_and_a_handoff_are_routed() {
    let req = |op: &str, args: serde_json::Value| agent::wire::Request {
        operation: op.into(),
        args: args.as_object().cloned().unwrap_or_default(),
    };
    let note = control::route(
        &req(
            "change.note",
            serde_json::json!({ "number": 79, "text": "I rebased onto !80" }),
        ),
        "s",
        1,
    );
    assert!(
        matches!(&note, Ok(Routed::Note { number: 79, text }) if text == "I rebased onto !80"),
        "{note:?}"
    );
    let hand = control::route(
        &req(
            "change.handoff",
            serde_json::json!({ "number": 79, "session": "https://claude.ai/code/session_b" }),
        ),
        "s",
        1,
    );
    assert!(
        matches!(&hand, Ok(Routed::Handoff { number: 79, session, .. }) if session.ends_with("session_b")),
        "{hand:?}"
    );
    assert!(
        control::route(
            &req(
                "change.handoff",
                serde_json::json!({ "number": 79, "session": " " })
            ),
            "s",
            1
        )
        .is_err(),
        "names a session"
    );
}

#[test]
fn a_grant_operation_names_its_grant() {
    for op in ["grant.request", "grant.rotate", "grant.revoke"] {
        assert_eq!(
            control::route(&request(op, &json!({ "name": "vapid" })), "a", 1),
            Ok(Routed::Grant {
                operation: op.into(),
                name: "vapid".into()
            })
        );
        assert!(control::route(&request(op, &json!({ "name": "Bad Name" })), "a", 1).is_err());
        assert!(control::route(&request(op, &json!({})), "a", 1).is_err());
    }
    assert_eq!(
        control::route(&request("grant.list", &json!({})), "a", 1),
        Ok(Routed::Grant {
            operation: "grant.list".into(),
            name: String::new()
        })
    );
}

#[test]
fn a_proposal_naming_a_brief_may_leave_title_plan_and_scope_to_it() {
    let routed = control::route(
        &request(
            "change.propose",
            &json!({ "slug": "b4-covers", "brief": "b4" }),
        ),
        "agent",
        1,
    );
    match routed {
        Ok(Routed::Propose {
            brief,
            title,
            scope,
            ..
        }) => {
            assert_eq!(brief.as_deref(), Some("b4"));
            assert!(title.is_empty() && scope.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let without = control::route(
        &request("change.propose", &json!({ "slug": "x" })),
        "agent",
        1,
    );
    assert!(
        without.is_err(),
        "title, plan and scope are needed without a brief"
    );
}

fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_deploy_main_moved_past_is_superseded_before_its_builds() {
    let t = tempfile::tempdir().expect("tempdir");
    let (remote, work, job) = (
        t.path().join("r.git"),
        t.path().join("w"),
        t.path().join("j"),
    );
    for d in [&work, &job] {
        std::fs::create_dir_all(d).expect("dir");
    }
    git_in(
        t.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().expect("utf8"),
        ],
    );
    git_in(&work, &["init", "-q", "-b", "main"]);
    git_in(&work, &["commit", "-q", "--allow-empty", "-m", "a"]);
    let a = git_in(&work, &["rev-parse", "HEAD"]);
    git_in(
        &work,
        &["push", "-q", remote.to_str().expect("utf8"), "main"],
    );
    let remote = remote.to_str().expect("utf8");
    let spec = |sha: &str| {
        agent::control::deploy_spec(remote, "", sha, None, std::collections::BTreeMap::new())
    };
    assert_eq!(
        agent::ship::superseded(&spec(&a), &job),
        None,
        "main is still there"
    );
    git_in(&work, &["commit", "-q", "--allow-empty", "-m", "b"]);
    let b = git_in(&work, &["rev-parse", "HEAD"]);
    git_in(&work, &["push", "-q", remote, "main"]);
    let why = agent::ship::superseded(&spec(&a), &job).expect("superseded");
    assert!(why.contains(&b), "{why}");
    assert_eq!(
        agent::ship::superseded(&spec(&a), &t.path().join("missing")),
        None,
        "an unreadable head never stops a deploy"
    );
}

#[test]
fn a_passed_deploy_replaces_an_older_release_target() {
    let mut r = control::Releasing::default();
    r.deployed("a");
    r.deployed("b");
    assert_eq!(r.start().as_deref(), Some("b"), "the newest deploy only");
    assert_eq!(r.start(), None);
}

#[test]
fn a_deploy_passed_while_a_release_runs_waits_for_it() {
    let mut r = control::Releasing::default();
    r.deployed("a");
    assert_eq!(r.start().as_deref(), Some("a"));
    r.deployed("b");
    r.deployed("c");
    assert_eq!(r.start(), None, "one release at a time");
    r.finished(false, "boom", 7);
    let last = r.last.clone().expect("last");
    assert_eq!((last.sha.as_str(), last.passed), ("a", false));
    assert_eq!(r.start().as_deref(), Some("c"));
    r.finished(true, "nothing to release", 8);
    assert_eq!(
        r.start(),
        None,
        "a release isn't retried until the next passed deploy"
    );
}

#[test]
fn a_release_without_a_container_waits_unless_a_newer_deploy_came() {
    let mut r = control::Releasing::default();
    r.deployed("a");
    let a = r.start().expect("starts");
    r.wait(&a);
    assert_eq!(r.start().as_deref(), Some("a"));
    r.deployed("b");
    r.wait("a");
    assert_eq!(r.start().as_deref(), Some("b"));
}

#[test]
fn a_release_target_waits_from_its_oldest_unreleased_deploy() {
    let mut r = control::Releasing::default();
    r.deployed_at("a", 10);
    r.deployed_at("b", 50);
    assert_eq!(r.waited(70), 60);
    assert_eq!(r.start().as_deref(), Some("b"));
    r.finished(true, "ok", 80);
    assert_eq!(r.waited(90), 0);
}

#[test]
fn a_deploy_during_a_release_waits_from_its_end() {
    let mut r = control::Releasing::default();
    r.deployed_at("a", 10);
    r.start();
    r.deployed_at("b", 20);
    r.finished(true, "ok", 40);
    assert_eq!(r.waited(100), 60);
}

#[test]
fn a_change_holding_the_merge_turn_wakes_every_10_seconds() {
    let mut c = Change::propose(14, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    assert_eq!(control::alarm_secs(&c, 20), 20);
    c.merge_after = Some("window:1".into());
    assert_eq!(control::alarm_secs(&c, 20), agent::actions::POLL_TURN);
    assert_eq!(
        control::alarm_secs(&c, 5),
        5,
        "never later than its own poll"
    );
}

#[test]
fn a_journal_entry_appends_once() {
    use agent::core::journal;
    let db = rusqlite::Connection::open_in_memory().expect("sqlite");
    db.execute_batch(include_str!("../../../migrations/0008_journal.sql"))
        .expect("migrates");
    let e = agent::core::events::Event {
        id: 7,
        at: 100,
        kind: "change.checked".into(),
        change: None,
        session: None,
        summary: "checked".into(),
        owner: false,
        stage: None,
        model: None,
        subject: None,
        resolves: None,
    };
    let s = journal::event_entry("noisypigeon", &e, journal::SHADOW_EPOCH);
    run(&db, &s);
    run(&db, &s);
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 1);
    let row: (String, i64, i64) = db
        .query_row("SELECT object, seq, epoch FROM journal", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .expect("row");
    assert_eq!(row, ("instance:noisypigeon/events".to_string(), 7, 0));
}

#[test]
fn a_changed_key_is_journaled_once() {
    use agent::core::journal;
    use serde_json::json;
    use std::collections::BTreeMap;
    let mut now = BTreeMap::new();
    now.insert("members".to_string(), json!({"a": 1}));
    now.insert("tasks".to_string(), json!([]));
    let first = journal::changed(&BTreeMap::new(), &now);
    let keys: Vec<&str> = first.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["members", "tasks"]);
    let previous: BTreeMap<String, String> = first.into_iter().collect();
    assert_eq!(
        journal::changed(&previous, &now),
        Vec::<(String, String)>::new()
    );
    now.insert("members".to_string(), json!({"a": 2}));
    let second = journal::changed(&previous, &now);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].0, "members");
    now.remove("tasks");
    let third = journal::changed(&previous, &now);
    assert!(third.contains(&("tasks".to_string(), String::new())));
    let db = rusqlite::Connection::open_in_memory().expect("sqlite");
    db.execute_batch(include_str!("../../../migrations/0008_journal.sql"))
        .expect("migrates");
    let s = journal::snapshot_entry("noisypigeon", 1, 100, "members", &json!({"a": 2}));
    run(&db, &s);
    run(&db, &s);
    let rows: Vec<(String, String)> = db
        .prepare("SELECT object, kind FROM journal")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        rows,
        [(
            "instance:noisypigeon/state".to_string(),
            "members".to_string()
        )]
    );
}

#[test]
fn a_value_too_large_for_one_entry_is_not_journaled() {
    use agent::core::journal;
    use serde_json::json;
    use std::collections::BTreeMap;
    let mut now = BTreeMap::new();
    now.insert("small".to_string(), json!(1));
    now.insert("big".to_string(), json!("x".repeat(journal::MAX_VALUE)));
    let keys: Vec<String> = journal::changed(&BTreeMap::new(), &now)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, ["small"]);
}

#[test]
fn a_sealed_entry_opens_only_as_its_own_object_and_key() {
    use agent::core::journal;
    let seal = journal::seal_key(&"ab".repeat(32)).unwrap();
    let nonce = journal::nonce(&"0".repeat(24)).unwrap();
    let value = json!({"push": "tok"});
    let body = journal::sealed(&seal, nonce, "instance:x/state", "devices", &value).unwrap();
    assert_eq!(
        journal::unseal(&seal, "instance:x/state", "devices", &body).unwrap(),
        value
    );
    assert!(journal::unseal(&seal, "instance:x/state", "waiting", &body).is_err());
    assert!(journal::unseal(&seal, "instance:y/state", "devices", &body).is_err());
    let other = journal::seal_key(&"cd".repeat(32)).unwrap();
    assert!(journal::unseal(&other, "instance:x/state", "devices", &body).is_err());
    assert!(!body.to_string().contains("tok"));
    assert!(journal::seal_key("nope").is_err());
}

#[test]
fn a_failed_deploy_advances_only_the_groups_that_passed() {
    let mut p = Pipeline::default();
    p.pushed(A);
    assert_eq!(p.start().as_deref(), Some(A));
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(p.baselines["build:pigeon"], A);
    assert_eq!(p.baselines["build:judge"], A);
    assert_eq!(p.baselines["sites"], A);
    assert_eq!(p.baselines["workers"], A);
    p.pushed(B);
    assert_eq!(p.start().as_deref(), Some(B));
    let failed = Report {
        passed: vec!["sites".into()],
        ..deploy_report(Outcome::Failed)
    };
    p.finished(&failed, 2);
    assert_eq!(p.deployed.as_deref(), Some(A));
    assert_eq!(
        p.baselines_spec(),
        vec![
            format!("build:judge={A}"),
            format!("build:pigeon={A}"),
            format!("sites={B}"),
            format!("workers={A}")
        ]
    );
}

#[test]
fn a_retry_deploys_a_named_group_in_full() {
    let mut p = Pipeline::default();
    p.pushed(A);
    assert_eq!(p.start().as_deref(), Some(A));
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(p.retry(Some("nope")), None);
    assert_eq!(p.running, None, "an unknown unit starts nothing");
    assert_eq!(p.retry(Some("workers")).as_deref(), Some(A));
    assert_eq!(p.baselines["workers"], "");
    assert_eq!(p.baselines["sites"], A);
    assert_eq!(p.baselines["build:pigeon"], A);
}

#[test]
fn a_group_reads_its_own_baseline() {
    let mut spec = control::deploy_spec("r", "t", B, Some(A), std::collections::BTreeMap::new());
    spec.scope = vec![
        "workers=".into(),
        format!("sites={B}"),
        format!("build:judge={B}"),
    ];
    assert_eq!(control::group_since(&spec, "workers"), None);
    assert_eq!(control::group_since(&spec, "sites").as_deref(), Some(B));
    assert_eq!(
        control::group_since(&spec, "build:pigeon").as_deref(),
        Some(A)
    );
    assert_eq!(
        control::group_since(&spec, "build:judge").as_deref(),
        Some(B)
    );
}
