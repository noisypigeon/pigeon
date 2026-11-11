//! `scope::jobs` against this repository: a change runs only the checks its paths need; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::path::{Path, PathBuf};

use cobot::scope::{Job, jobs};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..")
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
    assert_eq!(
        names(&[
            ".config/policies/layout.yaml",
            "src/pigeon-cli/crates/sentinel/src/rules.rs"
        ])
        .iter()
        .filter(|n| *n == "test this repository's policies")
        .count(),
        1,
        "once, beside sentinel's own tests"
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
            "test cobot (catalog manifest apex git credentials)",
            "policy.check"
        ]
    );
}

#[test]
fn cobot_runs_itself_its_dependants_and_the_worker() {
    let got = names(&["src/pigeon-cli/crates/cobot/src/git.rs"]);
    assert!(
        got.contains(&"clippy cobot pigeon pigeon-backplane".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"test cobot pigeon pigeon-backplane".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy pigeon --features cobot".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy pigeon --features backplane".to_string()),
        "{got:?}"
    );
    assert!(
        !got.contains(&"clippy pigeon --features sentinel".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy cobot worker (wasm)".to_string()),
        "{got:?}"
    );
    assert!(
        !got.iter()
            .any(|n| n.contains("sentinel ") || n == "infra.validate"),
        "{got:?}"
    );
    assert_eq!(got.last().map(String::as_str), Some("policy.check"));
}

#[test]
fn sentinel_runs_only_its_own_checks_as_nothing_links_it() {
    let got = names(&["src/pigeon-cli/crates/sentinel/src/rules.rs"]);
    assert_eq!(
        got,
        [
            "fmt",
            "clippy pigeon-sentinel",
            "test pigeon-sentinel",
            "policy.check"
        ],
        "pigeon and backplane run pigeon-sentinel, so they don't rebuild for it; see \
         .claude/decisions/2026-11-10-development-factory-crates-functional-core.md §1"
    );
}

#[test]
fn a_pigeon_command_runs_only_its_feature() {
    let got = names(&["src/pigeon-cli/crates/pigeon/src/commands/cobot.rs"]);
    assert!(got.contains(&"test pigeon".to_string()), "{got:?}");
    let features: Vec<&String> = got.iter().filter(|n| n.contains("--features")).collect();
    assert_eq!(features, ["clippy pigeon --features cobot"], "{got:?}");
}

#[test]
fn pigeons_sentinel_command_runs_its_feature_apart_from_the_release_units() {
    let got = names(&["src/pigeon-cli/crates/pigeon/src/commands/sentinel.rs"]);
    let features: Vec<&String> = got.iter().filter(|n| n.contains("--features")).collect();
    assert_eq!(features, ["clippy pigeon --features sentinel"], "{got:?}");
}

#[test]
fn a_workspace_setting_runs_everything() {
    let got = names(&["src/pigeon-cli/Cargo.lock"]);
    for want in [
        "fmt",
        "clippy",
        "clippy pigeon (no features)",
        "clippy pigeon --features sentinel",
        "clippy pigeon --features cobot",
        "clippy pigeon --features backplane",
        "clippy backplane worker (wasm)",
        "test",
        "test cobot worker",
        "backplane instance",
        "policy.check",
    ] {
        assert!(got.contains(&want.to_string()), "{want}: {got:?}");
    }
    assert_eq!(names(&[".config/releases.yaml"]), got);
}

#[test]
fn site_templates_build_the_sites() {
    assert_eq!(
        names(&["src/site-templates/blog/index.html"]),
        ["sites.build", "policy.check"]
    );
}

#[test]
fn deploys_name_the_sites_a_change_to_main_publishes() {
    let root = repo_root();
    let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let all = cobot::scope::deploys(&root, None).expect("finds");
    assert_eq!(all, ["sites/docs"], "only the instance's manifest sites");
    let got = cobot::scope::deploys(
        &root,
        Some(&files(&["catalog/cloudflare/capabilities.yaml"])),
    )
    .expect("finds");
    assert_eq!(
        got,
        ["sites/docs"],
        "the architecture site draws the catalog"
    );
    let got = cobot::scope::deploys(&root, Some(&files(&["config.yaml"]))).expect("finds");
    assert!(got.contains(&"sites/docs".to_string()), "{got:?}");
    let got = cobot::scope::deploys(
        &root,
        Some(&files(&["workloads/noisypigeon.com/blog/site/posts/x.md"])),
    )
    .expect("finds");
    assert_eq!(
        got,
        ["sites/docs"],
        "the architecture draws workloads too; the legacy blog publishes from its own instance"
    );
    let got = cobot::scope::deploys(
        &root,
        Some(&files(&[
            "src/pigeon-cli/crates/cobot/src/git.rs",
            "CLAUDE.md",
        ])),
    )
    .expect("finds");
    assert_eq!(
        got,
        Vec::<String>::new(),
        "code and docs publish nothing; a template reaches a site by its pinned tag"
    );
}

#[test]
fn workers_deploy_when_their_spec_crates_or_migrations_change() {
    let workers = |files: Option<&[&str]>| {
        let files: Option<Vec<String>> =
            files.map(|f| f.iter().map(|s| (*s).to_string()).collect());
        cobot::scope::workers(&repo_root(), files.as_deref()).expect("scopes")
    };
    let control = vec!["pigeon-control".to_string()];
    for hit in [
        "src/pigeon-cli/crates/backplane/src/mcp.rs",
        "src/pigeon-cli/crates/cobot/src/workers.rs",
        "src/pigeon-cli/crates/backplane/migrations/0003_x.sql",
        "infra/pigeon/workers/control/worker.yaml",
        "src/pigeon-cli/Dockerfile",
        "src/pigeon-cli/Cargo.lock",
    ] {
        assert_eq!(workers(Some(&[hit])), control, "{hit}");
    }
    for miss in [
        ".claude/decisions/x.md",
        "src/site-templates/blog/templates/index.html",
        "infra/pigeon/sites/docs/config.yaml",
    ] {
        assert!(workers(Some(&[miss])).is_empty(), "{miss}");
    }
    assert_eq!(workers(None), control);
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
    assert_eq!(
        spikes[0].argv,
        ["sh", "../../tooling/spikes/buildkit/run.sh"]
    );
    assert_eq!(
        names(&["tooling/README.md"]),
        ["policy.check"],
        "other tooling runs no spike"
    );
}

#[test]
fn the_full_set_is_what_a_workspace_wide_change_runs() {
    let full: Vec<String> = cobot::scope::full(&repo_root())
        .expect("reads")
        .into_iter()
        .map(|j| j.name)
        .collect();
    for want in ["fmt", "clippy", "test", "clippy backplane worker (wasm)"] {
        assert!(full.contains(&want.to_string()), "{want}: {full:?}");
    }
    let wide = names(&["src/pigeon-cli/Cargo.lock"]);
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
        cobot::scope::test_map().argv,
        ["bash", "../../tooling/ci/test-map.sh"],
        "as backplane's actions::on_github knows it"
    );
    assert!(!names(&["src/pigeon-cli/Cargo.lock"]).contains(&"test map".to_string()));
}

const COBOT: &str = "src/pigeon-cli/crates/cobot/";

fn map() -> cobot::scope::TestMap {
    let bin = |package: &str, kind: &str, name: &str, files: &[&str]| cobot::scope::Binary {
        package: package.into(),
        kind: kind.into(),
        name: name.into(),
        files: files.iter().map(|f| format!("{COBOT}{f}")).collect(),
    };
    let mut binaries = vec![
        bin("cobot", "lib", "cobot", &["src/git.rs", "src/scope.rs"]),
        bin("cobot", "test", "git", &["src/git.rs"]),
        bin(
            "cobot",
            "test",
            "scope",
            &["src/scope.rs", "src/release.rs"],
        ),
    ];
    for n in 0..2 {
        binaries.push(bin("pigeon", "test", &format!("t{n}"), &["src/other.rs"]));
    }
    cobot::scope::TestMap {
        sha: "a".repeat(40),
        binaries,
    }
}

fn with_map(files: &[&str]) -> Vec<String> {
    let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
    cobot::scope::jobs_with(&repo_root(), &files, Some(&map()))
        .expect("scopes")
        .into_iter()
        .map(|Job { name, .. }| name)
        .collect()
}

fn tests_of(names: &[String]) -> Vec<&String> {
    names
        .iter()
        .filter(|n| n.starts_with("test ") && *n != "test cobot worker")
        .collect()
}

#[test]
fn the_test_map_runs_the_binaries_that_ran_a_changed_file() {
    let got = with_map(&["src/pigeon-cli/crates/cobot/src/git.rs"]);
    assert_eq!(tests_of(&got), ["test cobot: lib git"], "{got:?}");
    assert!(
        got.contains(&"clippy cobot pigeon pigeon-backplane".to_string()),
        "clippy still covers the dependants: {got:?}"
    );
    let got = with_map(&["src/pigeon-cli/crates/cobot/tests/scope.rs"]);
    assert_eq!(tests_of(&got), ["test cobot: scope"], "its own binary");
    let jobs = cobot::scope::jobs_with(
        &repo_root(),
        &["src/pigeon-cli/crates/cobot/src/release.rs".to_string()],
        Some(&map()),
    )
    .expect("scopes");
    let test = jobs
        .iter()
        .find(|j| j.name.starts_with("test cobot:"))
        .expect("a test");
    assert_eq!(
        test.argv,
        [
            "cargo", "test", "-p", "cobot", "--test", "scope", "--locked"
        ]
    );
}

#[test]
fn the_test_map_falls_back_to_crates_when_it_cannot_say() {
    let by_crate = ["test cobot pigeon pigeon-backplane"];
    for (files, why) in [
        (
            &["src/pigeon-cli/crates/cobot/src/new.rs"][..],
            "a file it doesn't know",
        ),
        (
            &["src/pigeon-cli/crates/cobot/Cargo.toml"][..],
            "a manifest",
        ),
        (
            &["src/pigeon-cli/crates/cobot/tests/state/x.json"][..],
            "a fixture",
        ),
        (
            &[
                "src/pigeon-cli/crates/cobot/src/git.rs",
                "src/pigeon-cli/crates/cobot/src/scope.rs",
            ][..],
            "more than half of the binaries",
        ),
    ] {
        let got = with_map(files);
        assert_eq!(tests_of(&got), by_crate, "{why}: {got:?}");
    }
    let got = names(&["src/pigeon-cli/crates/cobot/src/git.rs"]);
    assert_eq!(tests_of(&got), by_crate, "no map");
}

#[test]
fn a_mapped_test_check_does_not_stand_in_for_a_crates_whole_suite() {
    let got = with_map(&[
        "src/pigeon-cli/crates/cobot/src/git.rs",
        "catalog/cloudflare/kinds/git-repo/schema.json",
    ]);
    assert!(
        got.contains(&"test cobot (catalog manifest apex git credentials)".to_string()),
        "{got:?}"
    );
}

#[test]
fn no_check_name_holds_a_comma_which_a_github_cache_key_refuses() {
    for files in [
        &["src/pigeon-cli/crates/cobot/src/git.rs"][..],
        &["src/pigeon-cli/Cargo.lock"][..],
        &["catalog/cloudflare/kinds/git-repo/schema.json"][..],
    ] {
        for name in with_map(files) {
            assert!(!name.contains(','), "{name}");
        }
    }
}
