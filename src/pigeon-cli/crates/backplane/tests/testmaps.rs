//! `main`'s test map: dispatched on each mirrored commit, its map copied when it passes; see the
//! 2026-11-11 amendment to `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.

use backplane::actions::{Due, Mirror};
use backplane::testmaps::{self, TestMaps};
use serde_json::json;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn running(t: &mut TestMaps, run: u64) {
    assert_eq!(t.actions.due(Mirror::Passed, 0), [Due::Dispatch(0)]);
    t.actions.dispatched(0, 0);
    let title = format!("!0 {A} ({})", t.actions.dispatches[0].id);
    let listed = json!({ "workflow_runs": [{ "id": run, "display_title": title }] });
    t.actions.found(0, &listed, 1);
}

fn finish(t: &mut TestMaps, conclusion: &str) {
    let run = json!({ "status": "completed" });
    let jobs = json!({ "jobs": [{
        "name": "test map", "status": "completed", "conclusion": conclusion, "id": 9
    }] });
    for f in t.actions.polled(0, &run, &jobs, 100) {
        t.finished(0, &f);
    }
    t.prune();
}

#[test]
fn a_mirrored_commit_is_mapped_and_its_map_copied() {
    let mut t = TestMaps::default();
    t.mirrored(A);
    assert!(t.busy());
    running(&mut t, 42);
    finish(&mut t, "success");
    assert_eq!(t.copying, Some((A.to_string(), 42)));
    assert_eq!(t.actions.dispatches.len(), 0);
    t.copied(A, "ab");
    assert!(!t.busy());
    assert_eq!(t.latest_map.as_ref().map(|m| m.sha.as_str()), Some(A));
    t.mirrored(A);
    assert!(!t.busy(), "a mapped commit isn't mapped again");
}

#[test]
fn a_failed_map_waits_for_the_next_commit_and_keeps_the_last() {
    let mut t = TestMaps::default();
    t.copied(B, "cd");
    t.mirrored(A);
    t.decided(A, true);
    running(&mut t, 42);
    finish(&mut t, "failure");
    assert!(!t.busy());
    assert!(t.failed.as_deref().is_some_and(|f| f.contains(A)));
    assert_eq!(t.latest_map.as_ref().map(|m| m.sha.as_str()), Some(B));
    assert_eq!(t.failed_sha.as_deref(), Some(A), "a task is opened for it");
}

#[test]
fn a_newer_commit_replaces_an_older_ones_dispatch() {
    let mut t = TestMaps::default();
    t.mirrored(A);
    t.mirrored(B);
    assert_eq!(t.actions.dispatches.len(), 1);
    assert_eq!(t.actions.dispatches[0].sha, B);
    let body = backplane::actions::dispatch_body(
        &t.actions.dispatches[0],
        testmaps::MAIN,
        &[testmaps::check()],
    );
    assert_eq!(body["inputs"]["number"], "0");
    assert_eq!(
        body["inputs"]["checks"],
        r#"[{"name":"test map","argv":["bash","../../tooling/ci/test-map.sh"]}]"#
    );
}

#[test]
fn only_the_map_of_the_commit_is_kept() {
    let map = format!(r#"{{"version":1,"sha":"{A}","binaries":[]}}"#);
    assert_eq!(testmaps::invalid(map.as_bytes(), A), None);
    assert!(testmaps::invalid(map.as_bytes(), B).is_some());
    assert!(testmaps::invalid(b"PK\x03\x04rest", A).is_some_and(|w| w.contains("zipped")));
    assert!(testmaps::invalid(b"not json", A).is_some());
    assert_eq!(testmaps::key(A), format!("builds/{A}/tests.json"));
}

const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn a_commit_with_no_rust_input_keeps_the_newest_map() {
    let mut t = TestMaps::default();
    t.copied(B, "cd");
    t.mirrored(A);
    assert!(t.busy(), "it waits to be compared");
    assert_eq!(t.actions.dispatches.len(), 0);
    t.decided(A, false);
    assert!(!t.busy());
    assert_eq!(t.skipped.as_deref(), Some(A));
    assert_eq!(t.latest_map.as_ref().map(|m| m.sha.as_str()), Some(B));
    t.mirrored(C);
    t.decided(C, true);
    assert_eq!(t.actions.dispatches.len(), 1, "a Rust change is mapped");
    assert_eq!(t.actions.dispatches[0].sha, C);
}

#[test]
fn rust_inputs_are_sources_manifests_and_toolchains() {
    let has = |f: &str| testmaps::rust_inputs(&[f.to_string()]);
    assert!(has("src/pigeon-cli/crates/backplane/src/testmaps.rs"));
    assert!(has("src/pigeon-cli/Cargo.lock"));
    assert!(has("src/pigeon-cli/crates/pigeon/Cargo.toml"));
    assert!(has("src/pigeon-cli/.cargo/config.toml"));
    assert!(has("mise.toml"));
    assert!(!has(
        ".claude/decisions/2026-11-10-development-factory-crates-functional-core.md"
    ));
    assert!(!has("src/pigeon-ios/Pigeon/ContentView.swift"));
    let files = |n: usize| json!({ "files": (0..n).map(|i| json!({ "filename": format!("docs/{i}.md") })).collect::<Vec<_>>() });
    assert!(!testmaps::rust_since(&files(3)));
    assert!(
        testmaps::rust_since(&files(testmaps::COMPARE_FILES)),
        "a cut listing counts"
    );
    assert!(testmaps::rust_since(&json!({})), "an unreadable one counts");
}

#[test]
fn a_squash_names_its_change() {
    assert_eq!(
        testmaps::merged_change("feat(x): y (!241)\n\nbody"),
        Some(241)
    );
    assert_eq!(testmaps::merged_change("chore: release"), None);
}
