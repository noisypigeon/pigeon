//! D1 schema v0 applies to SQLite and enforces its constraints; see the control-plane ADR §4,
//! `.claude/decisions/2026-11-05-runtime-control-plane-worker.md`.

use rusqlite::Connection;

const SCHEMA_V0: &str = include_str!("../migrations/0001_schema_v0.sql");

fn applied() -> Connection {
    let db = Connection::open_in_memory().expect("sqlite");
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    db.execute_batch(SCHEMA_V0).expect("schema v0 applies");
    db.execute_batch(
        "INSERT INTO instances (id, name, created) VALUES ('i1', 'noisypigeon', 1);
         INSERT INTO plans (id, instance, version, digest, summary, requester, created, expires)
           VALUES ('p1', 'i1', 1, 'd', '{\"units\":[]}', 'owner', 1, 86401);",
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
