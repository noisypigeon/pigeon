//! The deploy judgements and the test map's transitions as tables, with no storage or disk.

use std::collections::{BTreeMap, BTreeSet};

use super::scope::{Binary, Crates, InfraTests, Layout, TestMap, Unit, jobs_of};
use super::ship::{healthy, whole_deploy};
use super::testmaps::{Map, TestMaps};
use crate::release::Config;

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

/// The workspace of the reach cases: `pigeon-agent`, with a suite, depending on `pigeon-core`.
fn reach_crates() -> Crates {
    let s = |v: &str| v.to_string();
    Crates {
        by_dir: [
            (s("src/pigeon-agent/"), s("pigeon-agent")),
            (s("src/pigeon-core/"), s("pigeon-core")),
        ]
        .into(),
        deps: [(s("pigeon-agent"), [s("pigeon-core")].into())].into(),
        bins: BTreeMap::default(),
        features: BTreeMap::default(),
        tests: [(s("pigeon-agent"), [s("suite")].into())].into(),
        layout: Layout {
            workspace: String::new(),
            worker: None,
            policy_tests: Vec::new(),
            infra_tests: None,
            ..Layout::default()
        },
    }
}

/// The map of the reach cases: `pigeon-core`'s lib, and `pigeon-agent`'s lib and three suite
/// modules, one of them nested.
fn reach_map() -> TestMap {
    let unit = |package: &str, kind: &str, name: &str, filter: &str, files: &[&str]| Binary {
        package: package.into(),
        kind: kind.into(),
        name: name.into(),
        filter: filter.into(),
        files: files.iter().map(|f| (*f).to_string()).collect(),
    };
    let restack = "src/pigeon-core/src/change/restack.rs";
    let jobs = "src/pigeon-agent/src/jobs.rs";
    let suite = |filter: &str, files: &[&str]| unit("pigeon-agent", "test", "suite", filter, files);
    TestMap {
        sha: A.into(),
        binaries: vec![
            unit("pigeon-core", "lib", "pigeon_core", "", &[restack]),
            unit("pigeon-core", "test", "e1", "", &["elsewhere.rs"]),
            unit("pigeon-core", "test", "e2", "", &["elsewhere.rs"]),
            unit("pigeon-core", "test", "e3", "", &["elsewhere.rs"]),
            unit("pigeon-agent", "lib", "agent", "", &[jobs]),
            suite(
                "merges::",
                &["src/pigeon-agent/tests/suite/merges.rs", restack],
            ),
            suite("jobs::", &["src/pigeon-agent/tests/suite/jobs.rs", jobs]),
            suite("merge::queue::", &["src/pigeon-agent/src/queue.rs"]),
        ],
    }
}

fn reach_config() -> Config {
    Config::parse("description: t\nrepository: https://example.test/r\nunits: {}\ntiers: []\n")
        .expect("config")
}

/// The units of the check named `name` that selection gives a change touching `files`.
fn units(crates: &Crates, files: &[&str], name: &str) -> Vec<Unit> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    jobs_of(&reach_config(), crates, &files, Some(&reach_map()))
        .into_iter()
        .find(|j| j.name == name)
        .map(|j| j.units)
        .unwrap_or_default()
}

#[test]
fn each_test_check_records_the_units_it_runs() {
    let crates = reach_crates();
    let agent = "src/pigeon-agent/src/jobs.rs";
    assert_eq!(
        units(&crates, &[agent], "test pigeon-agent: lib"),
        [Unit::target("pigeon-agent", "lib", "agent", "")],
        "whole targets"
    );
    assert_eq!(
        units(
            &crates,
            &[agent, "src/pigeon-agent/src/queue.rs"],
            "test pigeon-agent: suite jobs merge::queue"
        ),
        [
            Unit::target("pigeon-agent", "test", "suite", "jobs::"),
            Unit::target("pigeon-agent", "test", "suite", "merge::queue::"),
        ],
        "a suite's modules, filters kept whole"
    );
    assert_eq!(
        units(
            &crates,
            &["src/pigeon-core/src/new.rs"],
            "test pigeon-agent pigeon-core"
        ),
        [Unit::whole("pigeon-agent"), Unit::whole("pigeon-core")],
        "the crate fallback, with its dependants"
    );
    let modules = vec!["infra::".to_string(), "infra_remote::".to_string()];
    let mut infra = reach_crates();
    infra.layout.infra_tests = Some(InfraTests {
        package: "pigeon-agent".into(),
        test: "suite".into(),
        modules: modules.clone(),
    });
    assert_eq!(
        units(
            &infra,
            &["infra/pigeon/unit.yaml"],
            "test pigeon-agent: suite infra infra_remote"
        ),
        modules
            .iter()
            .map(|m| Unit::target("pigeon-agent", "test", "suite", m))
            .collect::<Vec<_>>(),
        "the infra job, as suite modules"
    );
    infra.tests = [(
        "pigeon-agent".to_string(),
        BTreeSet::from(["infra".to_string(), "infra_remote".to_string()]),
    )]
    .into();
    assert_eq!(
        units(
            &infra,
            &["infra/pigeon/unit.yaml"],
            "test pigeon-agent: infra infra_remote"
        ),
        [
            Unit::target("pigeon-agent", "test", "infra", ""),
            Unit::target("pigeon-agent", "test", "infra_remote", ""),
        ],
        "the infra job, as its own targets"
    );
}
