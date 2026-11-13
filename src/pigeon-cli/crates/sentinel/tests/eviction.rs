//! The `eviction` target: a candidate's fields judged one at a time, the first policy without
//! violations being the grant; see
//! `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4.

use sentinel::{Candidate, LoadedPolicy, Policy, PullRequest};
use tempfile::TempDir;

const SESSION_GONE: &str = r"description: A change whose session has gone two hours, idle as long, is evicted.
target: eviction
rules:
  - field: { name: session_live, equals: false }
  - field: { name: session_gone_for, at_least: 7200 }
  - field: { name: idle, at_least: 7200 }
  - field: { name: pinned, equals: false }
  - field: { name: approval_open, equals: false }
  - field: { name: queued, equals: false }
  - field: { name: state, not: '^(approved|merged|closed)$' }
";

const STALE_PROPOSED: &str = r"description: A change never authorized, idle a day, with no live session, is evicted.
target: eviction
rules:
  - field: { name: state, equals: proposed }
  - field: { name: session_live, equals: false }
  - field: { name: idle, at_least: 86400, at_most: 31536000 }
  - field: { name: pinned, equals: false }
";

fn loaded(name: &str, yaml: &str) -> LoadedPolicy {
    LoadedPolicy {
        name: name.into(),
        policy: Policy::parse(yaml).expect("valid"),
    }
}

fn policies() -> Vec<LoadedPolicy> {
    vec![
        loaded("eviction-stale-proposed", STALE_PROPOSED),
        loaded("eviction-session-gone", SESSION_GONE),
    ]
}

fn gone() -> Candidate {
    [
        ("number", "79"),
        ("kind", "change"),
        ("state", "pushed"),
        ("pushed", "true"),
        ("session_live", "false"),
        ("session_gone_for", "9000"),
        ("idle", "8000"),
        ("pinned", "false"),
        ("approval_open", "false"),
        ("queued", "false"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn with(mut c: Candidate, key: &str, value: &str) -> Candidate {
    c.insert(key.into(), value.into());
    c
}

#[test]
fn a_gone_sessions_idle_change_is_granted_by_its_policy() {
    let (verdicts, granted) = sentinel::evict(&policies(), &gone());
    assert_eq!(granted.as_deref(), Some("eviction-session-gone"));
    assert_eq!(verdicts.len(), 2, "every eviction policy answers");
    assert_eq!(
        verdicts[1].policy, "eviction-stale-proposed",
        "in name order"
    );
    assert_ne!(verdicts[1].violations, Vec::<String>::new());
}

#[test]
fn what_the_adr_protects_is_never_granted() {
    let table = [
        ("a live session", with(gone(), "session_live", "true")),
        ("gone too short", with(gone(), "session_gone_for", "7199")),
        ("pinned", with(gone(), "pinned", "true")),
        ("approved", with(gone(), "state", "approved")),
        (
            "an open approval page",
            with(gone(), "approval_open", "true"),
        ),
        ("in the merge queue", with(gone(), "queued", "true")),
        ("not a number", with(gone(), "idle", "soon")),
    ];
    for (name, c) in table {
        let (_, granted) = sentinel::evict(&policies(), &c);
        assert_eq!(granted, None, "{name}");
    }
    let mut missing = gone();
    missing.remove("pinned");
    let (verdicts, granted) = sentinel::evict(&policies(), &missing);
    assert_eq!(granted, None, "a missing field never grants");
    assert!(verdicts.iter().any(|v| {
        v.violations
            .iter()
            .any(|m| m.contains("`pinned` is not set"))
    }));
}

#[test]
fn numeric_bounds_hold_at_both_ends() {
    let proposed = with(with(gone(), "state", "proposed"), "idle", "86400");
    let (_, granted) = sentinel::evict(&[loaded("p", STALE_PROPOSED)], &proposed);
    assert_eq!(granted.as_deref(), Some("p"), "at the bound");
    let over = with(proposed.clone(), "idle", "31536001");
    assert_eq!(
        sentinel::evict(&[loaded("p", STALE_PROPOSED)], &over).1,
        None
    );
    let under = with(proposed, "idle", "86399");
    assert_eq!(
        sentinel::evict(&[loaded("p", STALE_PROPOSED)], &under).1,
        None
    );
}

#[test]
fn check_skips_eviction_policies() {
    let root = TempDir::new().expect("tempdir");
    let pr = PullRequest {
        branch: Some("change/1-x".into()),
        ..PullRequest::default()
    };
    let report = sentinel::check(&policies(), root.path(), Some(&pr)).expect("runs");
    assert_eq!((report.ran, report.skipped), (0, 2));
    assert_eq!(report.violations.len(), 0);
}

#[test]
fn eviction_fields_and_rules_stay_with_their_target() {
    let bad = [
        (
            "an eviction field on change",
            "description: x.\ntarget: change\nrules:\n  - field: { name: idle, at_least: 1 }\n",
        ),
        (
            "a pull-request field on eviction",
            "description: x.\ntarget: eviction\nrules:\n  - field: { name: title, matches: x }\n",
        ),
        (
            "two kinds at once",
            "description: x.\ntarget: eviction\nrules:\n  - field: { name: idle, at_least: 1, equals: 1 }\n",
        ),
        (
            "a document rule on eviction",
            "description: x.\ntarget: eviction\nrules:\n  - line: { at: 1, matches: x }\n",
        ),
    ];
    for (name, yaml) in bad {
        assert!(Policy::parse(yaml).is_err(), "{name}");
    }
    let numeric_on_change =
        "description: x.\ntarget: change\nrules:\n  - field: { name: number, at_most: 9999 }\n";
    assert!(
        Policy::parse(numeric_on_change).is_ok(),
        "the numeric rules apply anywhere"
    );
}
