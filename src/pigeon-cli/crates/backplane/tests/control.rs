//! The control plane's routing, push events and job reports; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1, §4.

use backplane::changes::{BOUND, Change, Check, Event};
use backplane::control::{self, Outcome, Pipeline, Push, Report, Routed, Target};
use backplane::wire::Request;
use serde_json::json;

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
        control::job_id(3, A, "clippy pigeon --features cobot"),
        "3-0123456789ab-clippy-pigeon---features-cobot"
    );
}

#[test]
fn reports_become_events() {
    let report = |outcome, violations: Vec<&str>| Report {
        id: "x".into(),
        outcome,
        log: String::new(),
        violations: violations.into_iter().map(Into::into).collect(),
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec!["cargo".into()],
        }],
        merged: None,
        published: vec![],
        workers: vec![],
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
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
        tiers: None,
    };
    let text = serde_json::to_string(&e).expect("serialize");
    assert!(text.contains("\"type\":\"bounded\""), "{text}");
    assert_eq!(
        serde_json::from_str::<Event>(&text).expect("deserialize"),
        e
    );
}

const SCHEMA: [&str; 3] = [
    include_str!("../migrations/0001_schema_v0.sql"),
    include_str!("../migrations/0002_changes.sql"),
    include_str!("../migrations/0003_change_sessions.sql"),
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
        checks: vec![Check {
            name: "fmt".into(),
            argv: vec![],
        }],
        tiers: None,
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
        checks: vec![],
        merged: None,
        published: vec!["sites/docs".into()],
        workers: vec!["pigeon-control: v1 → v2".into()],
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
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
fn the_newest_build_a_deploy_stored_is_what_jobs_run() {
    let build = |sha: &str| backplane::control::Build {
        sha: sha.into(),
        sha256: "ab".repeat(32),
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
fn a_lost_deploy_retries_then_fails() {
    let mut p = Pipeline::default();
    p.pushed(A);
    p.start();
    for _ in 1..backplane::runs::MAX_ATTEMPTS {
        assert_eq!(p.lost(1).as_deref(), Some(A));
    }
    assert_eq!(p.lost(1), None);
    assert_eq!(p.history[0].outcome, Outcome::Failed);
    assert!(p.running.is_none());
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
        backplane::control::Class::Runner
    );
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
    use backplane::changes::MERGE;
    use backplane::control::Class;
    assert_eq!(control::class_of(BOUND), Class::Runner);
    assert_eq!(control::class_of(MERGE), Class::Runner);
    assert_eq!(
        control::class_of(control::DEPLOY),
        Class::Checker,
        "a deploy builds the Worker"
    );
    assert_eq!(control::class_of("fmt"), Class::Checker);
    assert_eq!(
        control::class_of("clippy backplane worker (wasm)"),
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
    assert_eq!(p.retry(), None, "nothing deployed or failed yet");
    p.pushed(A);
    assert_eq!(p.start().as_deref(), Some(A));
    assert_eq!(p.retry(), None, "a deploy runs");
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(
        p.history[0].workers,
        vec!["pigeon-control: v1 → v2".to_string()]
    );
    assert_eq!(
        p.retry().as_deref(),
        Some(A),
        "a passed push is deployed again"
    );
    assert_eq!(p.deployed, None, "from nothing: everything is deployed");
    assert_eq!(p.retry(), None, "a deploy runs");
    p.finished(&deploy_report(Outcome::Passed), 1);
    assert_eq!(p.deployed.as_deref(), Some(A));
    p.pushed(B);
    assert_eq!(p.start().as_deref(), Some(B));
    p.finished(&deploy_report(Outcome::Failed), 2);
    assert_eq!(p.start(), None, "a failure waits");
    assert_eq!(p.retry().as_deref(), Some(B), "until it is retried");
    assert_eq!(p.retry(), None, "once at a time");
    p.finished(&deploy_report(Outcome::Passed), 3);
    assert_eq!(p.deployed.as_deref(), Some(B));
}

#[test]
fn deploys_reference_the_newest_images_and_the_owners_push_wins() {
    let imgs = |source: &str| backplane::control::Images {
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
