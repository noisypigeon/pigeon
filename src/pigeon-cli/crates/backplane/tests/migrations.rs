//! D1 schema v0 applies to SQLite and enforces its constraints; see the control-plane ADR §4,
//! `.claude/decisions/2026-11-05-runtime-control-plane-worker.md`.

#[path = "common/step.rs"]
mod step;

use rusqlite::Connection;
use step::Apply;

const SCHEMA_V0: &str = include_str!("../migrations/0001_schema_v0.sql");
const CHANGES: &str = include_str!("../migrations/0002_changes.sql");

fn applied() -> Connection {
    let db = Connection::open_in_memory().expect("sqlite");
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    db.execute_batch(SCHEMA_V0).expect("schema v0 applies");
    db.execute_batch(
        "INSERT INTO instances (id, name, created) VALUES ('i1', 'noisypigeon', 1);
         INSERT INTO instances (id, name, created) VALUES ('i9', 'another', 1);
         INSERT INTO plans (id, instance, version, digest, summary, requester, created, expires)
           VALUES ('p1', 'i1', 1, 'd', '{\"units\":[]}', 'owner', 1, 86401);
         INSERT INTO plans (id, instance, version, digest, summary, requester, created, expires)
           VALUES ('p9', 'i9', 1, 'd', '{}', 'owner', 1, 86401);",
    )
    .expect("seed");
    db
}

fn refused(db: &Connection, sql: &str) -> bool {
    db.execute_batch(sql).is_err()
}

#[test]
fn schema_v0_has_every_table_the_adr_names() {
    let db = applied();
    let mut names: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .expect("query")
        .query_map([], |r| r.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("names");
    names.sort();
    assert_eq!(
        names,
        [
            "approvals",
            "audit",
            "desired_state",
            "grants",
            "instances",
            "plans",
            "runs"
        ]
    );
}

#[test]
fn schema_v0_refuses_bad_rows() {
    let db = applied();
    assert!(
        refused(
            &db,
            "INSERT INTO runs (id, instance, kind) VALUES ('r1', 'i1', 'apply')"
        ),
        "an apply names a plan"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO runs (id, instance, kind, plan) VALUES ('r2', 'i1', 'plan', 'p1')"
        ),
        "a plan run names none"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO runs (id, instance, kind) VALUES ('r3', 'i1', 'destroy')"
        ),
        "an unknown kind"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO runs (id, instance, kind, plan) VALUES ('r4', 'nobody', 'apply', 'p1')"
        ),
        "an unknown instance"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO runs (id, instance, kind, plan) VALUES ('r7', 'i1', 'apply', 'p9')"
        ),
        "an apply names its own instance's plan"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO plans (id, instance, version, digest, summary, requester, created, expires)
               VALUES ('p2', 'i1', 1, 'd', '{}', 'owner', 10, 10)"
        ),
        "a plan expires after it is made"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO desired_state (instance, unit, version, config, author, created)
               VALUES ('i1', 'infra/x', 1, 'not json', 'owner', 1)"
        ),
        "desired state is JSON"
    );
    assert!(
        refused(
            &db,
            "INSERT INTO instances (id, name, created) VALUES ('i2', 'noisypigeon', 1)"
        ),
        "an instance's name is unique"
    );
    db.execute_batch(
        "INSERT INTO runs (id, instance, kind, plan) VALUES ('r5', 'i1', 'apply', 'p1');
         INSERT INTO runs (id, instance, kind) VALUES ('r6', 'i1', 'plan');
         INSERT INTO approvals (plan, approver, created) VALUES ('p1', 'owner', 2);
         INSERT INTO audit (at, actor, operation, instance, outcome) VALUES (3, 'owner', 'infra.apply', 'i1', 'ok');",
    )
    .expect("good rows");
    assert!(
        refused(
            &db,
            "INSERT INTO approvals (plan, approver, created) VALUES ('p1', 'owner', 4)"
        ),
        "one approval per approver"
    );
}

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn with_changes() -> Connection {
    let db = applied();
    db.execute_batch(CHANGES).expect("0002 applies");
    db.execute_batch(&format!(
        "INSERT INTO changes (instance, repo, number, slug, title, plan, scope, proposer, state,
           head, plan_approver, created, updated)
         VALUES ('i1', 'pigeon/pigeon', 7, 'x', 't', 'p', '[]', 'agent', 'bounded', '{SHA}',
           'owner', 1, 1);"
    ))
    .expect("seed");
    db
}

#[test]
fn changes_refuse_impossible_rows() {
    let db = with_changes();
    let insert = |state: &str, head: &str, plan_approver: &str, approver: &str, sha: &str| {
        format!(
            "INSERT INTO changes (instance, repo, number, slug, title, plan, scope, proposer, state,
               head, plan_approver, approver, approved_sha, created, updated)
             VALUES ('i1', 'pigeon/pigeon', 8, 'y', 't', 'p', '[]', 'agent', '{state}', {head},
               {plan_approver}, {approver}, {sha}, 1, 1);"
        )
    };
    let sha = format!("'{SHA}'");
    for (why, sql) in [
        (
            "unknown state",
            insert("done", "NULL", "'owner'", "NULL", "NULL"),
        ),
        (
            "pushed without a head",
            insert("pushed", "NULL", "'owner'", "NULL", "NULL"),
        ),
        (
            "planned without a plan approver",
            insert("planned", "NULL", "NULL", "NULL", "NULL"),
        ),
        (
            "self-approved plan",
            insert("planned", "NULL", "'agent'", "NULL", "NULL"),
        ),
        (
            "approved without an approver",
            insert("approved", &sha, "'owner'", "NULL", "NULL"),
        ),
        (
            "approved another SHA",
            insert(
                "approved",
                &sha,
                "'owner'",
                "'owner'",
                "'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'",
            ),
        ),
        (
            "self-approved change",
            insert("approved", &sha, "'owner'", "'agent'", &sha),
        ),
        (
            "short head",
            insert("pushed", "'abc'", "'owner'", "NULL", "NULL"),
        ),
    ] {
        assert!(refused(&db, &sql), "{why}");
    }
    assert!(!refused(
        &db,
        &insert("approved", &sha, "'owner'", "'owner'", &sha)
    ));
    let job = |change: u32, status: &str| {
        format!(
            "INSERT INTO jobs (instance, repo, change, sha, name, status, updated)
             VALUES ('i1', 'pigeon/pigeon', {change}, '{SHA}', 'fmt', '{status}', 1);"
        )
    };
    assert!(refused(&db, &job(99, "queued")), "a job needs its change");
    assert!(refused(&db, &job(7, "lost")), "unknown job status");
    assert!(!refused(&db, &job(7, "running")));
}

/// Every migration, in order, applied to SQLite.
fn migrated() -> Connection {
    let db = applied();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("migrations")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .filter(|p| !p.ends_with("0001_schema_v0.sql"))
        .collect();
    files.sort();
    for f in files {
        db.execute_batch(&std::fs::read_to_string(&f).expect("reads"))
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    }
    db
}

fn run(db: &Connection, statements: &[backplane::control::Statement]) {
    for s in statements {
        let params: Vec<rusqlite::types::Value> = s
            .params
            .iter()
            .map(|v| match v {
                serde_json::Value::Null => rusqlite::types::Value::Null,
                serde_json::Value::Number(n) => {
                    rusqlite::types::Value::Integer(n.as_i64().expect("integer"))
                }
                serde_json::Value::String(t) => rusqlite::types::Value::Text(t.clone()),
                other => rusqlite::types::Value::Text(other.to_string()),
            })
            .collect();
        db.execute(s.sql, rusqlite::params_from_iter(params))
            .unwrap_or_else(|e| panic!("{}: {e}", s.sql));
    }
}

#[test]
fn a_change_closed_before_its_plan_was_authorized_mirrors() {
    use backplane::changes::{Change, Event};
    use backplane::control;
    let db = migrated();
    let mut c = Change::propose(
        9,
        "reconcile-79",
        "fix: reconcile !79 with main",
        "p",
        vec!["a/**".into()],
        "pigeon",
    );
    let logs = serde_json::Map::new();
    run(&db, &control::mirror("i1", "pigeon", &c, &logs, 10));
    c.apply(Event::Close).expect("closes");
    run(&db, &control::mirror("i1", "pigeon", &c, &logs, 11));
    let (state, by): (String, String) = db
        .query_row(
            "SELECT state, plan_approver FROM changes WHERE number = 9",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(state, "closed");
    assert_eq!(by, control::CLOSED_UNAUTHORIZED);
    assert!(
        refused(
            &db,
            "UPDATE changes SET state = 'proposed' WHERE number = 9"
        ),
        "a proposed change still has no plan approver"
    );
}

#[test]
fn a_change_closed_after_its_approval_mirrors_without_an_approver() {
    use backplane::changes::{Change, Event};
    use backplane::control;
    let db = migrated();
    let mut c = Change::propose(191, "x", "refactor: x", "p", vec!["a/**".into()], "agent");
    let sha = "a".repeat(40);
    let events = [
        Event::ApprovePlan {
            by: "owner".into(),
            at: 1,
        },
        Event::Pushed { sha: sha.clone() },
        Event::Bounded {
            sha: sha.clone(),
            violations: vec![],
            checks: vec![],
            full: vec![],
            tiers: None,
            gate: None,
        },
        Event::Approve {
            by: "owner".into(),
            sha,
            at: 2,
        },
    ];
    for e in events {
        c.apply(e).expect("applies");
    }
    let logs = serde_json::Map::new();
    run(&db, &control::mirror("i1", "pigeon", &c, &logs, 10));
    c.apply(Event::Close).expect("closes");
    run(&db, &control::mirror("i1", "pigeon", &c, &logs, 11));
    let (state, by): (String, Option<String>) = db
        .query_row(
            "SELECT state, approver FROM changes WHERE number = 191",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!((state.as_str(), by), ("closed", None));
}
