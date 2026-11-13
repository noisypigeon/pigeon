//! Overlaps among the open changes; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §2.

use backplane::control::open_rows;
use backplane::sessions::{Open, overlaps, session};
use serde_json::json;

fn globs(list: &[&str]) -> Vec<String> {
    list.iter().map(|g| (*g).to_string()).collect()
}

fn open(number: u32, session: Option<&str>, scope: &[&str], affects: &[&str]) -> Open {
    Open {
        number,
        state: "bounded".into(),
        session: session.map(str::to_string),
        scope: globs(scope),
        affects: globs(affects),
        plan: String::new(),
        slug: String::new(),
    }
}

#[test]
fn a_proposal_sees_the_open_changes_it_meets() {
    let others = [
        open(
            60,
            Some("s-a"),
            &["src/pigeon-cli/crates/backplane/src/merge*.rs"],
            &[],
        ),
        open(61, Some("s-b"), &["infra/**"], &[]),
        open(
            62,
            None,
            &[".claude/decisions/2026-11-09-development-merge-queue.md"],
            &["src/pigeon-cli/crates/backplane/**"],
        ),
    ];
    let found = overlaps(
        &globs(&["src/pigeon-cli/crates/backplane/**"]),
        &[],
        &others,
    );
    let numbers: Vec<u32> = found.iter().map(|o| o.number).collect();
    assert_eq!(numbers, [60, 62], "infra/** is apart");
    assert_eq!(found[0].session.as_deref(), Some("s-a"));
    assert_eq!(found[1].globs, ["src/pigeon-cli/crates/backplane/**"]);
    assert!(found.iter().all(|o| o.same.is_empty()));
}

#[test]
fn an_adr_in_design_reserves_its_area_and_one_new_file_is_flagged() {
    let adr = ".claude/decisions/2026-11-09-development-merge-queue.md";
    let others = [open(70, Some("s-a"), &[adr], &["tooling/**"])];
    let by_affects = overlaps(&globs(&["tooling/spikes/x/**"]), &[], &others);
    assert_eq!(by_affects.len(), 1, "affects reserves tooling/**");
    let same = overlaps(&globs(&[adr]), &[], &others);
    assert_eq!(same[0].same, [adr]);
    assert_eq!(
        overlaps(&globs(&["docs/**"]), &globs(&["catalog/**"]), &others),
        []
    );
}

#[test]
fn a_session_label_is_one_short_line() {
    assert_eq!(session(None), Ok(None));
    assert_eq!(session(Some("  ")), Ok(None));
    assert_eq!(session(Some(" s-a ")), Ok(Some("s-a".into())));
    assert!(session(Some("a\nb")).is_err());
    assert!(session(Some(&"x".repeat(201))).is_err());
}

#[test]
fn open_rows_read_d1s_json_columns() {
    let rows = [
        json!({ "number": 5, "state": "checked", "session": "s-a", "scope": "[\"a/**\"]", "affects": "[]" }),
        json!({ "number": 6, "state": "proposed", "session": null, "scope": "[\"b\"]", "affects": "[\"c/**\"]" }),
        json!({ "number": "x", "state": "proposed" }),
    ];
    let open = open_rows(&rows);
    assert_eq!(open.len(), 2, "an unreadable row is skipped");
    assert_eq!(open[0].scope, ["a/**"]);
    assert_eq!(open[1].affects, ["c/**"]);
    assert_eq!(open[1].session, None);
}
