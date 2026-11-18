//! Runs this repository's real policies against fixtures; each rule must reject bad input.

use std::fs;
use std::path::{Path, PathBuf};

use eviction::{LoadedPolicy, PullRequest};
use tempfile::TempDir;

const URL: &str = "https://github.com/noisypigeon/pigeon";
const GOOD_NAME: &str = "2026-01-02-development-a-sample-decision.md";

fn policy(name: &str) -> LoadedPolicy {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.config/policies")
        .join(format!("{name}.yaml"));
    eviction::load_file(&path).expect("policy loads")
}

fn violations(name: &str, root: &Path, pr: Option<&PullRequest>) -> Vec<String> {
    eviction::check(&[policy(name)], root, pr)
        .expect("check runs")
        .violations
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn adr(date: &str, category: &str, title: &str) -> String {
    format!(
        "## {title}\n\n* Date: {date}\n* Model: Claude Opus 5.5\n* Author: Willow Graysen (@noisypigeon).\n* Category: {category}\n* Governs: `.claude/decisions/*.md`\n\n## Context\n\nWhy.\n\n```\n## Not a heading, it is inside a fence\n```\n\n## Implementation plan.\n\n### Steps\n\nHow.\n"
    )
}

fn good() -> String {
    adr("2026-01-02", "development", "A sample decision")
}

fn decisions_root(files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let dir = root.path().join(".claude/decisions");
    fs::create_dir_all(&dir).expect("mkdir");
    for (name, content) in files {
        fs::write(dir.join(name), content).expect("write");
    }
    root
}

#[track_caller]
fn passes(v: &[String]) {
    assert!(v.is_empty(), "expected no violations, got {v:#?}");
}

#[track_caller]
fn fails(v: &[String]) {
    assert!(!v.is_empty(), "expected a violation");
}

fn pr_root() -> TempDir {
    let root = decisions_root(&[(GOOD_NAME, &good())]);
    fs::write(
        root.path().join("CHANGELOG.md"),
        format!("# Changelog\n\n## 2026-01-02\n\n- ci: x ([#7]({URL}/pull/7))\n"),
    )
    .expect("write");
    root
}

#[test]
fn pull_request_skipped_without_one() {
    let report = eviction::check(&[policy("pull-request")], pr_root().path(), None).expect("runs");
    assert_eq!((report.ran, report.skipped), (0, 1));
}

#[test]
fn every_policy_is_valid() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.config/policies");
    let files = fs::read_dir(&dir)
        .expect("reads")
        .filter(|e| {
            e.as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "yaml"))
        })
        .count();
    assert_eq!(eviction::load_dir(&dir).expect("loads").len(), files);
}

fn tree_root(files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    for (path, content) in files {
        let p = root.path().join(path);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(p, content).expect("write");
    }
    root
}

const CI: &str = ".github/workflows/ci.yml";

const CI_YML: &str =
    "# Code checks; see .claude/decisions/2026-11-10-development-code-checks-github-actions.md.
name: ci
permissions:
  contents: read
on:
  workflow_dispatch:
    inputs:
      sha: { required: true, type: string }
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567
        with: { ref: \"${{ inputs.sha }}\" }
      - run: cargo fmt --all --check
";

/// Both mirror policies' violations on a tree of `files`.
fn mirror(files: &[(&str, &str)]) -> Vec<String> {
    let root = tree_root(files);
    let mut v = violations("mirror", root.path(), None);
    v.extend(violations("mirror-workflow", root.path(), None));
    v
}

#[test]
fn mirror_allows_only_a_dispatched_read_only_pinned_ci_yml() {
    passes(&mirror(&[]));
    passes(&mirror(&[(CI, CI_YML)]));
    fails(&mirror(&[
        (CI, CI_YML),
        (".github/workflows/ci.yml.yml", CI_YML),
    ]));

    let checkout = "      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567\n";
    for (why, content) in [
        (
            "a push trigger",
            CI_YML.replace("jobs:", "  push: { branches: [main] }\njobs:"),
        ),
        ("an inline trigger", CI_YML.replace("on:\n", "on: push\n")),
        (
            "a job's write permission",
            CI_YML.replace(
                "    runs-on:",
                "    permissions: { actions: write }\n    runs-on:",
            ),
        ),
        (
            "write-all",
            CI_YML.replace("  contents: read", "  contents: read\n  checks: write-all"),
        ),
        (
            "no permissions",
            CI_YML.replace("permissions:\n  contents: read\n", ""),
        ),
        (
            "a secret",
            CI_YML.replace("cargo fmt", "echo ${{ secrets.T }} && cargo fmt"),
        ),
        (
            "the token",
            CI_YML.replace("cargo fmt", "echo $GITHUB_TOKEN && cargo fmt"),
        ),
        (
            "a tag",
            CI_YML.replace(checkout, "      - uses: actions/checkout@v4\n"),
        ),
        (
            "no ref",
            CI_YML.replace(checkout, "      - uses: actions/checkout\n"),
        ),
        (
            "a short SHA",
            CI_YML.replace(checkout, "      - uses: actions/checkout@0123456\n"),
        ),
        (
            "a local action",
            CI_YML.replace(checkout, "      - uses: ./.github/actions/x\n"),
        ),
        (
            "a docker action",
            CI_YML.replace(checkout, "      - uses: docker://alpine:3\n"),
        ),
    ] {
        assert_ne!(content, CI_YML, "{why} changed nothing");
        assert!(
            !mirror(&[(CI, &content)]).is_empty(),
            "{why} should be refused"
        );
    }
}

#[test]
fn roles_holds_three_explicit_capability_lists_only() {
    let good = "# Each level's permissions.\nroles:\n  administrator: [read, propose, approve, approve.owner, auto, mandates, platform, members]\n  standard: []\n  service: [read, propose]\n";
    let check = |content: &str| {
        violations(
            "roles",
            tree_root(&[(".config/roles.yaml", content)]).path(),
            None,
        )
    };
    passes(&check(good));
    passes(&check(
        &good.replace("service: [read, propose]", "service: []"),
    ));
    passes(&violations("roles", tree_root(&[]).path(), None));
    for bad in [
        good.replace(", platform, members]", ", platform]"),
        good.replace(", members]", ", members, learner]"),
        good.replace("standard: []", "standard: [*]"),
        good.replace("standard: []", "standard: [read]"),
        good.replace("standard: []", "standard: [learner]"),
        good.replace("standard: []", "standard: [approve.owner]"),
        good.replace("standard: []", "user: []"),
        good.replace("service: [read, propose]", "service: [read, approve]"),
        good.replace("  standard: []\n", ""),
        good.replace("# Each", "Each"),
        good.replace("roles:\n", "members:\n"),
        format!("{good}  owner: [read]\n"),
    ] {
        fails(&check(&bad));
    }
}

#[test]
fn names_forbids_the_old_names_but_the_stored_ones_and_history() {
    // Spelled apart, so this file doesn't name them; see names.yaml.
    let infra = concat!("co", "bot");
    let agent = concat!("back", "plane");
    let judge = concat!("senti", "nel");
    let token = format!("{}_TOKEN", agent.to_uppercase());
    let adr = format!("2026-01-01-x-{judge}.md");
    let stored = format!("let k = \"{token}\";\n");
    let cited = format!("see {adr}\n");
    let state = format!("noisypigeon/pigeon-{infra}-state\n");
    let previous = format!("units:\n  - name: x\n    previous: {judge}\n");
    let history = format!("- {agent} v0.1.0\n");
    let ok = [
        ("src/a.rs", stored.as_str()),
        ("docs/a.md", cited.as_str()),
        ("tooling/a.sh", state.as_str()),
        (".config/releases.yaml", previous.as_str()),
        ("CHANGELOG.md", history.as_str()),
        (".claude/decisions/a.md", history.as_str()),
    ];
    passes(&violations("names", tree_root(&ok).path(), None));
    let word = format!("// {infra}\n");
    let beside = format!("let k = \"{token}\"; // {agent}\n");
    let run_on = format!("noisypigeon/pigeon-{infra}-states\n");
    let unit = format!("units:\n  - name: {infra}\n");
    let dir = format!("tooling/{judge}/a.sh");
    for (path, content) in [
        ("src/a.rs", word.as_str()),
        ("src/a.rs", beside.as_str()),
        ("tooling/a.sh", run_on.as_str()),
        (".config/releases.yaml", unit.as_str()),
        (dir.as_str(), ""),
    ] {
        let v = violations("names", tree_root(&[(path, content)]).path(), None);
        assert!(v.iter().any(|m| m.contains(path)), "{path}: {v:#?}");
    }
}

#[test]
fn every_policy_runs_its_examples() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/policies");
    let write = |path: &Path, text: &str| {
        fs::create_dir_all(path.parent().expect("parent"))?;
        fs::write(path, text)
    };
    let mut problems = Vec::new();
    let mut carrying = 0;
    for p in &eviction::load_dir(&dir).expect("policies load") {
        carrying += usize::from(p.policy.examples.is_some());
        let scratch = TempDir::new().expect("tempdir");
        problems.extend(eviction::examples(p, scratch.path(), &write).expect("examples run"));
    }
    assert!(carrying > 0, "no policy carries examples");
    assert!(problems.is_empty(), "{problems:#?}");
}
