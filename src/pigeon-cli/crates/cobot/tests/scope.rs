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
        .filter(|n| n.starts_with("test "))
        .count(),
        1,
        "once, when sentinel's and its dependants' tests already run"
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
            "test cobot (catalog, manifest, apex, git, credentials)",
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
fn sentinel_runs_sentinel_and_its_dependants_without_the_worker() {
    let got = names(&["src/pigeon-cli/crates/sentinel/src/rules.rs"]);
    assert!(
        got.contains(&"test pigeon pigeon-backplane sentinel".to_string()),
        "{got:?}"
    );
    assert!(!got.iter().any(|n| n.contains("cobot")), "{got:?}");
    assert!(
        got.contains(&"clippy pigeon --features sentinel".to_string()),
        "{got:?}"
    );
    assert!(
        got.contains(&"clippy backplane worker (wasm)".to_string()),
        "{got:?}"
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
fn a_workspace_setting_runs_everything() {
    let got = names(&["src/pigeon-cli/Cargo.lock"]);
    for want in [
        "fmt",
        "clippy",
        "clippy pigeon (no features)",
        "clippy pigeon --features sentinel",
        "clippy pigeon --features gantry",
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
