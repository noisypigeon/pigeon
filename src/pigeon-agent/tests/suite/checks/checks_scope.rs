//! `scope::jobs` against this repository: a change runs only the checks its paths need; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::path::{Path, PathBuf};

use agent::checks::scope::{Job, jobs};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The judge's crate: its directory and package, wherever the restructure has put it; see
/// `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` step 5.
fn judge() -> (String, String) {
    let dir = ["src/pigeon-eviction"]
        .into_iter()
        .find(|d| repo_root().join(d).join("Cargo.toml").is_file())
        .expect("the judge's crate");
    let manifest = std::fs::read_to_string(repo_root().join(dir).join("Cargo.toml")).expect("read");
    let package = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("name = "))
        .expect("a package name")
        .trim_matches('"')
        .to_string();
    (dir.to_string(), package)
}

fn names(files: &[&str]) -> Vec<String> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    jobs(&repo_root(), &files)
        .expect("scopes")
        .into_iter()
        .map(|Job { name, .. }| name)
        .collect()
}

#[test]
fn docs_only_runs_the_policies_only() {
    assert_eq!(
        names(&[".claude/decisions/x.md", "CLAUDE.md"]),
        ["policy.check"]
    );
    assert_eq!(
        names(&[".config/policies/layout.yaml"]),
        ["test this repository's policies", "policy.check"],
        "the tests that read this repository's policies run too"
    );
    let both = names(&[
        ".config/policies/layout.yaml",
        &format!("{}/src/rules.rs", judge().0),
    ]);
    assert!(
        both.iter()
            .filter(|n| *n == "test this repository's policies")
            .count()
            <= 1,
        "at most once: {both:?}"
    );
    assert!(
        both.iter().any(|n| n == "test this repository's policies"
            || (n.starts_with("test ") && n.contains(&judge().1) && n.contains("pigeon-agent"))),
        "the policies' tests run, alone or within the judge's and the agent's: {both:?}"
    );
    assert_eq!(names(&[]), ["policy.check"]);
}

#[test]
fn the_catalog_runs_infra_checks_and_no_rust_build() {
    let got = names(&["catalog/cloudflare/kinds/git-repo/schema.json"]);
    assert_eq!(
        got,
        [
            "infra.validate",
            "test pigeon-agent: suite infra infra_remote",
            "policy.check"
        ]
    );
}

#[test]
fn the_agents_engines_run_the_agent_its_dependants_and_the_worker() {
    let got = names(&["src/pigeon-agent/src/git/mod.rs"]);
    assert!(
        got.contains(&"clippy pigeon-agent pigeon-harness pigeon-ios".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"test pigeon-agent pigeon-harness pigeon-ios".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy pigeon-harness --features infra".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy pigeon-harness --features agent".to_string()),
        "{got:?}"
    );
    assert!(
        !got.contains(&"clippy pigeon-harness --features eviction".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy agent worker (wasm)".to_string()),
        "{got:?}"
    );
    assert!(
        !got.iter()
            .any(|n| n.contains("pigeon-eviction") || n == "infra.validate"),
        "{got:?}"
    );
    assert_eq!(got.last().map(String::as_str), Some("policy.check"));
}

// See `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`'s 2026-11-17
// amendment: pigeon-core links the judge's library for provider-call policies.
#[test]
fn eviction_runs_the_core_and_its_dependants_as_the_core_links_it() {
    let (dir, package) = judge();
    let got = names(&[&format!("{dir}/src/rules.rs")]);
    assert!(
        got.iter()
            .any(|n| n.starts_with("test ") && n.contains(&package) && n.contains("pigeon-core")),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy agent worker (wasm)".to_string()),
        "{got:?}"
    );
    assert_eq!(got.last().map(String::as_str), Some("policy.check"));
}

#[test]
fn a_pigeon_command_runs_the_features_that_build_it() {
    let got = names(&["src/pigeon-harness/src/commands/infra.rs"]);
    assert!(got.contains(&"test pigeon-harness".to_string()), "{got:?}");
    let features: Vec<&String> = got.iter().filter(|n| n.contains("--features")).collect();
    assert_eq!(
        features,
        [
            "clippy pigeon-harness --features agent",
            "clippy pigeon-harness --features infra"
        ],
        "agent's feature builds `pigeon infra` too: {got:?}"
    );
}

#[test]
fn pigeons_eviction_command_runs_its_feature_apart_from_the_release_units() {
    let got = names(&["src/pigeon-harness/src/commands/eviction.rs"]);
    let features: Vec<&String> = got.iter().filter(|n| n.contains("--features")).collect();
    assert_eq!(
        features,
        ["clippy pigeon-harness --features eviction"],
        "{got:?}"
    );
}

#[test]
fn a_workspace_setting_runs_everything() {
    let got = names(&["Cargo.lock"]);
    for want in [
        "fmt",
        "clippy",
        "clippy pigeon-harness (no features)",
        "clippy pigeon-harness --features eviction",
        "clippy pigeon-harness --features infra",
        "clippy pigeon-harness --features agent",
        "clippy agent worker (wasm)",
        "test",
        "pigeon instance",
        "policy.check",
    ] {
        assert!(got.contains(&want.to_string()), "{want}: {got:?}");
    }
    assert_eq!(names(&[".config/releases.yaml"]), got);
}

#[test]
fn the_ios_app_runs_its_macos_check_and_its_check_crate() {
    assert_eq!(
        names(&["src/pigeon-ios/Pigeon/ContentView.swift"]),
        [
            "fmt",
            "clippy pigeon-ios",
            "test pigeon-ios",
            "ios",
            "policy.check"
        ],
        "its check crate reads the Swift"
    );
    let ios = jobs(&repo_root(), &["src/pigeon-ios/project.yml".to_string()])
        .expect("scopes")
        .into_iter()
        .find(|j| j.name == "ios")
        .expect("an ios check");
    assert_eq!(ios.argv, ["bash", "../pigeon-ios/check.sh"]);
}

#[test]
fn site_templates_build_the_sites() {
    assert_eq!(
        names(&["src/pigeon-site-templates/blog/index.html"]),
        [
            "fmt",
            "clippy pigeon-site-templates",
            "test pigeon-site-templates",
            "sites.build",
            "policy.check"
        ]
    );
}

#[test]
fn deploys_name_the_sites_a_change_to_main_publishes() {
    let root = repo_root();
    let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let all = agent::checks::scope::deploys(&root, None).expect("finds");
    assert_eq!(all, ["sites/docs"], "only the instance's manifest sites");
    for changed in [
        &["catalog/cloudflare/capabilities.yaml"][..],
        &["config.yaml"],
        &["src/pigeon-agent/src/worker/mod.rs", "CLAUDE.md"],
        &["workloads/noisypigeon.com/blog/config.yaml"],
        &[".config/policies/layout.yaml"],
        &["src/pigeon-site-templates/architecture/templates/index.html"],
    ] {
        let got = agent::checks::scope::deploys(&root, Some(&files(changed))).expect("finds");
        assert_eq!(
            got,
            ["sites/docs"],
            "a change to what architecture.describe reads publishes it: {changed:?}"
        );
    }
    for changed in [
        &["workloads/noisypigeon.com/blog/site/posts/x.md"][..],
        &["src/pigeon-site-templates/blog/templates/index.html"],
        &["CLAUDE.md"],
        &["src/pigeon-harness/src/main.rs"],
        &[".claude/decisions/README.md"],
    ] {
        let got = agent::checks::scope::deploys(&root, Some(&files(changed))).expect("finds");
        assert!(
            got.is_empty(),
            "nothing architecture.describe reads changed: {changed:?}"
        );
    }
}

#[test]
fn workers_deploy_when_their_spec_crates_or_migrations_change() {
    let workers = |files: Option<&[&str]>| {
        let files: Option<Vec<String>> =
            files.map(|f| f.iter().map(|s| (*s).to_string()).collect());
        agent::checks::scope::workers(&repo_root(), files.as_deref()).expect("scopes")
    };
    let control = vec!["pigeon-control".to_string()];
    let both = vec!["pigeon-control".to_string(), "pigeon-front".to_string()];
    let front = vec!["pigeon-front".to_string()];
    for hit in [
        "src/pigeon-agent/src/mcp.rs",
        "src/pigeon-agent/src/workers/mod.rs",
        "src/pigeon-agent/migrations/0003_x.sql",
        "infra/pigeon/workers/control/worker.yaml",
        "Dockerfile",
    ] {
        assert_eq!(workers(Some(&[hit])), control, "{hit}");
    }
    assert_eq!(workers(Some(&["Cargo.lock"])), both, "Cargo.lock");
    for hit in [
        "src/pigeon-front/src/lib.rs",
        "infra/pigeon/workers/front/worker.yaml",
    ] {
        assert_eq!(workers(Some(&[hit])), front, "{hit}");
    }
    for miss in [
        ".claude/decisions/x.md",
        "src/pigeon-site-templates/blog/templates/index.html",
        "infra/pigeon/sites/docs/config.yaml",
        "mise.toml",
        ".config/releases.yaml",
    ] {
        assert!(workers(Some(&[miss])).is_empty(), "{miss}");
    }
    assert_eq!(workers(None), both);
}

#[test]
fn a_spike_runs_as_its_own_check() {
    let files: Vec<String> = [
        "tooling/spikes/buildkit/run.sh",
        "tooling/spikes/buildkit/README.md",
        "tooling/spikes/Bad_Name/run.sh",
        "tooling/README.md",
    ]
    .iter()
    .map(|f| (*f).to_string())
    .collect();
    let jobs = jobs(&repo_root(), &files).expect("scopes");
    let spikes: Vec<&Job> = jobs
        .iter()
        .filter(|j| j.name.starts_with("spike "))
        .collect();
    assert_eq!(spikes.len(), 1, "one check per well-named spike");
    assert_eq!(spikes[0].name, "spike buildkit");
    assert_eq!(spikes[0].argv, ["sh", "tooling/spikes/buildkit/run.sh"]);
    assert_eq!(
        names(&["tooling/README.md"]),
        ["policy.check"],
        "other tooling runs no spike"
    );
}

#[test]
fn the_full_set_is_what_a_workspace_wide_change_runs() {
    let full: Vec<String> = agent::checks::scope::full(&repo_root())
        .expect("reads")
        .into_iter()
        .map(|j| j.name)
        .collect();
    for want in ["fmt", "clippy", "test", "clippy agent worker (wasm)"] {
        assert!(full.contains(&want.to_string()), "{want}: {full:?}");
    }
    let wide = names(&["Cargo.lock"]);
    assert!(
        full.iter().all(|n| wide.contains(n)),
        "{full:?} within {wide:?}"
    );
}

#[test]
fn the_test_map_runs_when_its_script_changes() {
    assert_eq!(
        names(&["tooling/ci/test-map.sh"]),
        ["test map", "policy.check"]
    );
    assert_eq!(
        agent::checks::scope::test_map().argv,
        ["bash", "../../tooling/ci/test-map.sh"],
        "as the agent's actions::on_github knows it"
    );
    assert!(!names(&["Cargo.lock"]).contains(&"test map".to_string()));
}

const AGENT: &str = "src/pigeon-agent/";

fn map() -> agent::checks::scope::TestMap {
    let bin =
        |package: &str, kind: &str, name: &str, files: &[&str]| agent::checks::scope::Binary {
            package: package.into(),
            kind: kind.into(),
            name: name.into(),
            filter: String::new(),
            files: files.iter().map(|f| format!("{AGENT}{f}")).collect(),
        };
    let mut binaries = vec![
        bin(
            "pigeon-agent",
            "lib",
            "agent",
            &["src/git/mod.rs", "src/checks/scope.rs"],
        ),
        bin("pigeon-agent", "test", "git", &["src/git/mod.rs"]),
        bin(
            "pigeon-agent",
            "test",
            "checks_scope",
            &["src/checks/scope.rs", "src/release/mod.rs"],
        ),
    ];
    for n in 0..2 {
        binaries.push(bin("pigeon", "test", &format!("t{n}"), &["src/other.rs"]));
    }
    agent::checks::scope::TestMap {
        sha: "a".repeat(40),
        binaries,
    }
}

fn with_map(files: &[&str]) -> Vec<String> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    agent::checks::scope::jobs_with(&repo_root(), &files, Some(&map()))
        .expect("scopes")
        .into_iter()
        .map(|Job { name, .. }| name)
        .collect()
}

fn tests_of(names: &[String]) -> Vec<&String> {
    names.iter().filter(|n| n.starts_with("test ")).collect()
}

#[test]
fn the_test_map_runs_the_binaries_that_ran_a_changed_file() {
    let got = with_map(&["src/pigeon-agent/src/git/mod.rs"]);
    assert_eq!(
        tests_of(&got),
        ["test pigeon-agent: lib", "test pigeon-agent: suite git"],
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy pigeon-agent pigeon-harness pigeon-ios".to_string()),
        "clippy still covers the dependants: {got:?}"
    );
    let got = with_map(&["src/pigeon-agent/tests/checks_scope.rs"]);
    assert_eq!(
        tests_of(&got),
        ["test pigeon-agent: suite checks_scope"],
        "its own binary, now its suite module"
    );
    let jobs = agent::checks::scope::jobs_with(
        &repo_root(),
        &["src/pigeon-agent/src/release/mod.rs".to_string()],
        Some(&map()),
    )
    .expect("scopes");
    let test = jobs
        .iter()
        .find(|j| j.name.starts_with("test pigeon-agent:"))
        .expect("a test");
    assert_eq!(
        test.argv,
        [
            "cargo",
            "test",
            "-p",
            "pigeon-agent",
            "--test",
            "suite",
            "--locked",
            "--",
            "checks_scope::"
        ]
    );
}

#[test]
fn the_test_map_falls_back_to_crates_when_it_cannot_say() {
    let by_crate = ["test pigeon-agent pigeon-harness pigeon-ios"];
    for (files, why) in [
        (
            &["src/pigeon-agent/src/infra/new.rs"][..],
            "a file it doesn't know",
        ),
        (&["src/pigeon-agent/Cargo.toml"][..], "a manifest"),
        (&["src/pigeon-agent/tests/state/x.json"][..], "a fixture"),
        (
            &[
                "src/pigeon-agent/src/git/mod.rs",
                "src/pigeon-agent/src/checks/scope.rs",
            ][..],
            "more than half of the binaries",
        ),
    ] {
        let got = with_map(files);
        assert_eq!(tests_of(&got), by_crate, "{why}: {got:?}");
    }
    let got = names(&["src/pigeon-agent/src/git/mod.rs"]);
    assert_eq!(tests_of(&got), by_crate, "no map");
}

#[test]
fn a_mapped_test_check_does_not_stand_in_for_a_crates_whole_suite() {
    let got = with_map(&[
        "src/pigeon-agent/src/git/mod.rs",
        "catalog/cloudflare/kinds/git-repo/schema.json",
    ]);
    assert!(
        got.contains(&"test pigeon-agent: suite infra infra_remote".to_string()),
        "{got:?}"
    );
}

#[test]
fn no_check_name_holds_a_comma_which_a_github_cache_key_refuses() {
    for files in [
        &["src/pigeon-agent/src/git/mod.rs"][..],
        &["Cargo.lock"][..],
        &["catalog/cloudflare/kinds/git-repo/schema.json"][..],
    ] {
        for name in with_map(files) {
            assert!(!name.contains(','), "{name}");
        }
    }
}

#[test]
fn a_wasm_only_path_runs_fmt_and_the_wasm_clippy_alone() {
    // !476: only `worker/passkeys.rs`, which no native build compiles.
    let worker = "src/pigeon-agent/src/worker/passkeys.rs";
    let wasm_only = ["fmt", "clippy agent worker (wasm)", "policy.check"];
    assert_eq!(names(&[worker]), wasm_only, "no map");
    assert_eq!(with_map(&[worker]), wasm_only, "with a map");
    let got = with_map(&[worker, "src/pigeon-agent/src/git/mod.rs"]);
    assert_eq!(
        tests_of(&got),
        ["test pigeon-agent: lib", "test pigeon-agent: suite git"],
        "the native file's tests, chosen by the map: {got:?}"
    );
    assert_eq!(
        got.iter()
            .filter(|n| *n == "clippy agent worker (wasm)")
            .count(),
        1,
        "{got:?}"
    );
}

#[test]
fn a_unit_test_module_the_map_doesnt_know_runs_its_lib() {
    // !415: `core/change/tests.rs`, whose lines coverage doesn't list.
    for f in [
        "src/pigeon-agent/src/core/change/tests.rs",
        "src/pigeon-agent/src/core/tests/x.rs",
    ] {
        let got = with_map(&[f]);
        assert_eq!(tests_of(&got), ["test pigeon-agent: lib"], "{f}: {got:?}");
    }
}

fn suite_map() -> agent::checks::scope::TestMap {
    let unit = |package: &str, kind: &str, name: &str, filter: &str, files: &[&str]| {
        agent::checks::scope::Binary {
            package: package.into(),
            kind: kind.into(),
            name: name.into(),
            filter: filter.into(),
            files: files.iter().map(|f| format!("{AGENT}{f}")).collect(),
        }
    };
    let mut binaries = vec![
        unit("pigeon-agent", "lib", "agent", "", &["src/git/mod.rs"]),
        unit(
            "pigeon-agent",
            "test",
            "suite",
            "git::",
            &["src/git/mod.rs", "tests/suite/git.rs"],
        ),
        unit(
            "pigeon-agent",
            "test",
            "suite",
            "checks_scope::",
            &["src/checks/scope.rs"],
        ),
        unit(
            "pigeon-agent",
            "test",
            "suite",
            "merges::",
            &["src/merges.rs"],
        ),
    ];
    for n in 0..3 {
        binaries.push(unit(
            "pigeon",
            "test",
            &format!("t{n}"),
            "",
            &["src/other.rs"],
        ));
    }
    agent::checks::scope::TestMap {
        sha: "a".repeat(40),
        binaries,
    }
}

fn with_suite_map(files: &[&str]) -> Vec<Job> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    agent::checks::scope::jobs_with(&repo_root(), &files, Some(&suite_map()))
        .expect("scopes")
        .into_iter()
        .filter(|j| j.name.starts_with("test "))
        .collect()
}

#[test]
fn a_map_by_module_runs_only_the_suite_modules_a_change_reaches() {
    let got = with_suite_map(&["src/pigeon-agent/src/checks/scope.rs"]);
    assert_eq!(
        got.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(),
        ["test pigeon-agent: suite checks_scope"],
        "{got:?}"
    );
    assert_eq!(
        got[0].argv,
        [
            "cargo",
            "test",
            "-p",
            "pigeon-agent",
            "--test",
            "suite",
            "--locked",
            "--",
            "checks_scope::"
        ]
    );
    for (files, want, why) in [
        (
            &["src/pigeon-agent/src/git/mod.rs"][..],
            &["test pigeon-agent: lib", "test pigeon-agent: suite git"][..],
            "a whole lib and a module are two checks",
        ),
        (
            &["src/pigeon-agent/tests/suite/merges.rs"][..],
            &["test pigeon-agent: suite merges"][..],
            "a module file picks its module",
        ),
        (
            &["src/pigeon-agent/tests/suite/merges/replays.rs"][..],
            &["test pigeon-agent: suite merges"][..],
            "a module directory's file picks its module",
        ),
        (
            &["src/pigeon-agent/tests/suite/harness/http.rs"][..],
            &["test pigeon-agent: suite checks_scope git merges"][..],
            "the harness picks every module",
        ),
        (
            &["src/pigeon-agent/tests/suite/main.rs"][..],
            &["test pigeon-agent: suite checks_scope git merges"][..],
            "main.rs picks every module",
        ),
    ] {
        let got = with_suite_map(files);
        assert_eq!(
            got.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(),
            want,
            "{why}: {got:?}"
        );
    }
    let unknown = "src/pigeon-agent/tests/suite/brand_new.rs";
    let got = with_suite_map(&[unknown]);
    let crates: Vec<String> = names(&[unknown])
        .into_iter()
        .filter(|n| n.starts_with("test "))
        .collect();
    assert_eq!(
        got.iter().map(|j| j.name.clone()).collect::<Vec<_>>(),
        crates,
        "a module the map doesn't know falls back to crates"
    );
    assert!(
        crates.iter().any(|n| n.contains("pigeon-agent")),
        "{crates:?}"
    );
}

#[test]
fn a_map_without_filters_reads_as_whole_targets() {
    let map: agent::checks::scope::TestMap = serde_json::from_str(
        r#"{"version":1,"sha":"a","binaries":[{"package":"p","kind":"test","name":"x","argv":["cargo"],"files":["f.rs"]}]}"#,
    )
    .expect("a version 1 map reads");
    assert_eq!(map.binaries[0].filter, "", "{map:?}");
}

/// A deploy releases the iOS app only when it changed the app's own files, and the paths that
/// say so exist here, so a move never silently stops the releases; see the release ADR's
/// 2026-11-16 amendment.
#[test]
fn a_deploy_releases_the_ios_app_only_when_it_changed_it() {
    use agent::control::Report;
    use agent::iosrelease::{INPUTS, NOT_INPUTS, triggers};
    let root = repo_root();
    for p in INPUTS.iter().chain(NOT_INPUTS) {
        let (dir, name) = p.rsplit_once('/').expect("a path");
        let found = std::fs::read_dir(root.join(dir))
            .expect("its directory")
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with(name));
        assert!(found, "{p} names nothing in this repository");
    }
    let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    for app in [
        "src/pigeon-ios/project.yml",
        "src/pigeon-ios/archive.sh",
        "src/pigeon-ios/ci.sh",
        "tooling/ci/buildkite/ios-sign.sh",
    ] {
        assert!(root.join(app).is_file(), "{app}");
        assert!(triggers(Some(&files(&["CLAUDE.md", app]))), "{app}");
    }
    assert!(!triggers(Some(&files(&[
        "src/pigeon-ios/README.md",
        "src/pigeon-agent/src/worker/main_run.rs",
    ]))));
    let report = Report {
        ios: true,
        ..Report::of("deploy-x", agent::control::Outcome::Passed, "")
    };
    let wire = serde_json::to_value(&report).expect("serializes");
    assert_eq!(wire["ios"], true, "the Worker reads it");
    let back: Report = serde_json::from_value(wire).expect("parses");
    assert!(back.ios);
    let older: Report = serde_json::from_value(serde_json::json!({
        "id": "deploy-y", "outcome": "passed"
    }))
    .expect("an older job's report");
    assert!(!older.ios, "an older job never releases");
}

fn plan_of(files: &[&str]) -> Vec<String> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    agent::checks::scope::plan(&repo_root(), &files).expect("plans")
}

#[test]
fn a_deploy_plan_names_each_unit_its_paths_deploy() {
    let at = |plan: &[String], unit: &str| plan.iter().position(|u| u == unit);

    let worker = plan_of(&["src/pigeon-agent/src/mcp.rs"]);
    let (w, s, b) = (
        at(&worker, "worker:pigeon-control"),
        at(&worker, "site:sites/docs"),
        at(&worker, "build:pigeon"),
    );
    assert!(w.is_some() && s.is_some() && b.is_some(), "{worker:?}");
    assert!(
        w < s && s < b,
        "workers, then sites, then the build: {worker:?}"
    );

    let cli = plan_of(&["src/pigeon-harness/src/main.rs"]);
    assert!(
        !cli.contains(&"site:sites/docs".to_string()),
        "the harness isn't what the architecture site draws: {cli:?}"
    );
    assert!(cli.contains(&"build:pigeon".to_string()), "{cli:?}");
    assert!(
        !cli.contains(&"build:judge".to_string()),
        "the harness isn't the judge's crate: {cli:?}"
    );
    assert!(!cli.iter().any(|u| u.starts_with("worker:")), "{cli:?}");

    assert!(plan_of(&["Dockerfile"]).contains(&"images".to_string()));
    assert!(plan_of(&["src/pigeon-ios/App/x.swift"]).contains(&"ios".to_string()));

    let adr = plan_of(&[".claude/decisions/x.md"]);
    assert!(
        !adr.contains(&"site:sites/docs".to_string()),
        "an ADR isn't what the architecture site draws: {adr:?}"
    );
    assert!(!adr.iter().any(|u| u.starts_with("worker:")), "{adr:?}");
}
