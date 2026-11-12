//! The deploy judgements and the test map's transitions as tables, with no storage or disk.

use super::ship::{healthy, whole_deploy};
use super::testmaps::{Map, TestMaps};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// A case: its name, the changed files, whether images were rebuilt, the serving version, and
/// whether the deploy is whole.
type Case<'a> = (&'a str, Option<&'a [String]>, bool, Option<&'a str>, bool);

#[test]
fn a_deploy_is_whole_unless_only_shared_code_changed_since_a_serving_version() {
    let own = vec!["infra/pigeon/workers/control/worker.yaml".to_string()];
    let shared = vec!["src/pigeon-cli/crates/backplane/src/lib.rs".to_string()];
    let dir = "infra/pigeon/workers/control";
    let table: Vec<Case<'_>> = vec![
        ("no version serves", Some(&shared), false, None, true),
        ("images rebuilt", Some(&shared), true, Some("v1"), true),
        ("files unknown", None, false, Some("v1"), true),
        ("its own file changed", Some(&own), false, Some("v1"), true),
        ("shared code only", Some(&shared), false, Some("v1"), false),
    ];
    for (name, files, images, from, whole) in table {
        assert_eq!(
            whole_deploy(dir, files, images, from).is_some(),
            whole,
            "{name}"
        );
    }
}

#[test]
fn a_health_answer_counts_only_for_its_version() {
    let ok = serde_json::json!({ "ok": true, "version": "v2" });
    assert!(healthy(&ok, "v2"));
    assert!(!healthy(&ok, "v1"));
    assert!(!healthy(
        &serde_json::json!({ "ok": false, "version": "v2" }),
        "v2"
    ));
}

#[test]
fn the_test_map_transitions_answer_the_next_maps_and_leave_the_old_ones() {
    let empty = TestMaps::default();
    let queued = empty.mirrored(A);
    assert_eq!(empty, TestMaps::default(), "the old maps are unchanged");
    assert!(queued.busy(), "the first commit is mapped");
    let mapped = TestMaps {
        latest_map: Some(Map {
            sha: A.into(),
            sha256: "ab".into(),
        }),
        ..TestMaps::default()
    };
    let deciding = mapped.mirrored(B);
    assert_eq!(
        deciding.deciding.as_deref(),
        Some(B),
        "a later commit waits to be compared"
    );
    let skipped = deciding.decided(B, false);
    assert_eq!(
        (skipped.deciding, skipped.skipped.as_deref()),
        (None, Some(B))
    );
    assert!(deciding.decided(B, true).busy(), "a Rust change is mapped");
    let failed = mapped.copy_failed(B, "gone");
    assert!(failed.failed.is_some_and(|f| f.contains("gone")));
    assert!(mapped.failed.is_none());
}
