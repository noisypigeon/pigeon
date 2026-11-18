//! Runs this repository's real policies against fixtures; each rule must reject bad input.

use std::fs;
use std::path::{Path, PathBuf};

use eviction::{LoadedPolicy, PullRequest};
use tempfile::TempDir;

const URL: &str = "https://github.com/noisypigeon/pigeon";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
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

fn decisions(name: &str, content: &str) -> Vec<String> {
    let root = decisions_root(&[(name, content)]);
    violations("decisions", root.path(), None)
}

#[track_caller]
fn passes(v: &[String]) {
    assert!(v.is_empty(), "expected no violations, got {v:#?}");
}

#[track_caller]
fn fails(v: &[String]) {
    assert!(!v.is_empty(), "expected a violation");
}

#[test]
fn decisions_valid() {
    passes(&decisions(GOOD_NAME, &good()));
    passes(&decisions(
        "2026-01-02-development-one-two-three-four-five.md",
        &good(),
    ));
}

#[test]
fn decisions_filename() {
    for name in [
        "2026-01-02-design-a-sample-decision.md",
        "2026-01-02-development-two-words.md",
        "2026-01-02-development-one-two-three-four-five-six.md",
        "2026-01-02-development-A-sample-decision.md",
    ] {
        fails(&decisions(name, &good()));
    }
}

#[test]
fn decisions_metadata() {
    let g = good();
    for content in [
        adr("2026-01-03", "development", "A sample decision"),
        adr("2026-01-02", "security", "A sample decision"),
        adr("2026-01-02", "development", "Title"),
        g.replace("Willow Graysen (@noisypigeon).", "Someone Else."),
        g.replace("* Author: Willow Graysen (@noisypigeon).\n", ""),
        g.replace("Model: Claude Opus 5.5", "Model:"),
        g.replace(
            "* Date: 2026-01-02\n* Model: Claude Opus 5.5",
            "* Model: Claude Opus 5.5\n* Date: 2026-01-02",
        ),
        format!("---\ntitle: x\n---\n{g}"),
    ] {
        fails(&decisions(GOOD_NAME, &content));
    }
}

#[test]
fn decisions_sections() {
    let g = good();
    for content in [
        g.replace("## Context\n", ""),
        g.replace("## Implementation plan.\n", ""),
        g.replace("## Context", "## TMP")
            .replace("## Implementation plan.", "## Context")
            .replace("## TMP", "## Implementation plan."),
        format!("{g}\n## Consequences\n\nMore.\n"),
        format!("{g}\n# Stray\n"),
    ] {
        fails(&decisions(GOOD_NAME, &content));
    }
}

#[test]
fn decisions_amendments() {
    let g = good();
    let amend = "\n## Amendments\n\n### 2026-01-02: First\n\n#### Detail\n\nText.\n\n### 2026-02-01: Second\n";
    passes(&decisions(GOOD_NAME, &format!("{g}{amend}")));
    for tail in [
        "\n## Amendments\n\n### 2026-01-01: Too early\n",
        "\n## Amendments\n\n### 2026-02-01: Later\n\n### 2026-01-01: Too early\n",
        "\n## Amendments\n\n### Second thoughts\n",
        "\n## Amendments\n\n### 2026-01-03:\n",
        "\n## Amendments\n\nNothing.\n",
        "\n## Amendments\n\n### 2026-01-03: x\n\n## Consequences\n\nMore.\n",
    ] {
        fails(&decisions(GOOD_NAME, &format!("{g}{tail}")));
    }
    fails(&decisions(
        GOOD_NAME,
        &g.replace(
            "## Implementation plan.",
            "## Amendments\n\n### 2026-01-03: Early\n\n## Implementation plan.",
        ),
    ));
}

#[test]
fn decisions_amendments_land_in_merge_order() {
    // A review dated before midnight UTC, merged after an entry dated the next day.
    let merged = "\n## Amendments\n\n### 2026-11-17: Merged first\n\nText.\n";
    let review = "\n### 2026-11-16: Review !839 against !831\n\nText.\n";
    let g = adr("2026-11-08", "development", "A sample decision");
    let name = "2026-11-08-development-a-sample-decision.md";
    passes(&decisions(name, &format!("{g}{merged}")));
    passes(&decisions(name, &format!("{g}{merged}{review}")));
}

#[test]
fn decisions_directory() {
    fails(&violations("decisions", decisions_root(&[]).path(), None));
    fails(&violations(
        "decisions",
        decisions_root(&[("README.md", "# Decisions\n")]).path(),
        None,
    ));
    passes(&violations(
        "decisions",
        decisions_root(&[(GOOD_NAME, &good()), ("README.md", "# Decisions\n")]).path(),
        None,
    ));
    fails(&violations(
        "decisions",
        decisions_root(&[(GOOD_NAME, &good()), ("notes.txt", "x\n")]).path(),
        None,
    ));
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

fn pr(title: &str, body: &str, number: Option<&str>) -> Vec<String> {
    pr_files(title, body, number, Some("src/x.rs\nREADME.md"))
}

fn pr_files(title: &str, body: &str, number: Option<&str>, files: Option<&str>) -> Vec<String> {
    let root = pr_root();
    let pr = PullRequest {
        title: Some(title.into()),
        body: Some(body.into()),
        number: number.map(Into::into),
        files: files.map(Into::into),
        ..PullRequest::default()
    };
    violations("pull-request", root.path(), Some(&pr))
}

#[test]
fn pull_request() {
    let cite = format!(".claude/decisions/{GOOD_NAME}");
    passes(&pr(
        "ci: enforce the lifecycle",
        &format!("## Decision\n\n{cite}"),
        Some("7"),
    ));
    passes(&pr("feat(cli)!: break things", &cite, Some("7")));
    for (title, body, number) in [
        ("Enforce the lifecycle", cite.clone(), Some("7")),
        ("feature: x", cite.clone(), Some("7")),
        ("ci: ", cite.clone(), Some("7")),
        ("ci: x", "No decision here.".into(), Some("7")),
        ("ci: x", ".claude/decisions/README.md".into(), Some("7")),
        (
            "ci: x",
            ".claude/decisions/2026-01-02-development-does-not-exist.md".into(),
            Some("7"),
        ),
        (
            "ci: x",
            format!("{cite} and .claude/decisions/2026-01-02-other-not-here-at-all.md"),
            Some("7"),
        ),
        ("ci: x", cite.clone(), Some("")),
        ("ci: x", cite.clone(), None),
        ("ci: x", cite.clone(), Some("7; true")),
        ("ci: x\"; exit 0; \"", "$(exit 0) `true`".into(), Some("7")),
    ] {
        fails(&pr(title, &body, number));
    }
    for files in [
        Some("CHANGELOG.md"),
        Some("src/a.rs\nsrc/pigeon-cli/crates/retired/CHANGELOG.md"),
        None,
    ] {
        fails(&pr_files("ci: x", &cite, Some("7"), files));
    }
    passes(&pr_files(
        "ci: x",
        &cite,
        Some("7"),
        Some("docs/CHANGELOG.md.bak\nsrc/a.rs"),
    ));
}

#[test]
fn pull_request_skipped_without_one() {
    let report = eviction::check(&[policy("pull-request")], pr_root().path(), None).expect("runs");
    assert_eq!((report.ran, report.skipped), (0, 1));
}

fn changelog(content: Option<&str>) -> Vec<String> {
    let root = TempDir::new().expect("tempdir");
    if let Some(c) = content {
        fs::write(root.path().join("CHANGELOG.md"), c).expect("write");
    }
    violations("changelog", root.path(), None)
}

fn cl_good() -> String {
    format!(
        "# Changelog\n\n## 2026-01-03\n\n### Released\n\n- a v0.1.1\n- b v0.2.0\n\n### Merged\n\n- feat: first ([#1]({URL}/pull/1)) · a\n- ci: second ([#2]({URL}/pull/2))\n- refactor: rename (!7) ([0123456]({URL}/commit/{SHA})) · a, b\n- units renamed · c → b\n\n## 2026-01-02\n\n### Merged\n\n- docs(adr): bootstrap ([0123456]({URL}/commit/{SHA}))\n"
    )
}

#[test]
fn changelog_rules() {
    let g = cl_good();
    passes(&changelog(Some(&g)));
    passes(&changelog(None));
    for content in [
        g.replace("# Changelog", "# Changes"),
        g.replace("2026-01-02", "2026-01-04"),
        g.replace("2026-01-02", "2026-01-03"),
        g.replace("## 2026-01-02", "## Jan 2"),
        format!("{g}\n## 2026-01-01\n"),
        "# Changelog\n".into(),
        g.replace(&format!(" ([#1]({URL}/pull/1))"), ""),
        g.replace(
            &format!("{URL}/pull/1"),
            "https://github.com/other/repo/pull/1",
        ),
        g.replace("[#1]", "[#3]"),
        g.replace(
            &format!("[#2]({URL}/pull/2)"),
            &format!("[#1]({URL}/pull/1)"),
        ),
        g.replace(&format!("commit/{SHA}"), "commit/0123456"),
        g.replace("[0123456]", "[fedcba9]"),
        format!("{g}  continued\n"),
        g.replace("# Changelog\n", "# Changelog\n\nIntro.\n"),
        g.replace(" · a\n", " · A\n"),
        g.replace("### Merged\n\n- feat", "### Notes\n\n- feat"),
    ] {
        fails(&changelog(Some(&content)));
    }
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

fn layout_root(skip: &[&str], extra: &[&str]) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let dirs = [
        ".claude",
        ".config",
        ".github",
        ".git",
        "src/pigeon-core",
        "tooling",
        "catalog",
        "infra",
        "workloads",
    ];
    let files = [
        ".gitignore",
        "CHANGELOG.md",
        "CLAUDE.md",
        "tooling/README.md",
        "catalog/README.md",
        "infra/README.md",
        "workloads/README.md",
    ];
    for d in dirs.iter().filter(|d| !skip.contains(d)) {
        fs::create_dir_all(root.path().join(d)).expect("mkdir");
    }
    for f in files.iter().chain(extra).filter(|f| !skip.contains(f)) {
        if f.ends_with('/') {
            fs::create_dir_all(root.path().join(f)).expect("mkdir");
        } else {
            fs::write(root.path().join(f), "").expect("write");
        }
    }
    root
}

fn layout(skip: &[&str], extra: &[&str]) -> Vec<String> {
    violations("layout", layout_root(skip, extra).path(), None)
}

#[test]
fn layout_rules() {
    passes(&layout(&[], &[]));
    passes(&layout(
        &[],
        &[
            "tooling/garden/",
            "infra/notes.md",
            "src/second-project/",
            "workloads/noisypigeon.com/",
            "catalog/cloudflare/",
        ],
    ));
    for (skip, extra) in [
        (&[][..], &["stray/"][..]),
        (&[], &["stray.txt"]),
        (&["workloads", "workloads/README.md"], &[]),
        (&[], &["src/README.md"]),
        (&[], &["src/Bad_Name/"]),
        (&["infra/README.md"], &[]),
        (&["src/pigeon-core"], &[]),
        (&[], &["workloads/notes.md"]),
        (&[], &["workloads/Not_A_Zone/"]),
        (&[], &["workloads/cloudflare/"]),
        (&["catalog", "catalog/README.md"], &[]),
        (&["catalog/README.md"], &[]),
        (&[], &["catalog/notes.md"]),
        (&[], &["catalog/Not_A_Provider/"]),
    ] {
        fails(&layout(skip, extra));
    }
}

/// The layout policy's verdict on a tree with `path` holding `content`.
fn layout_with(path: &str, content: &str) -> Vec<String> {
    let root = layout_root(&[], &[]);
    let file = root.path().join(path);
    fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
    fs::write(&file, content).expect("write");
    violations("layout", root.path(), None)
}

#[test]
fn layout_keeps_the_core_pure() {
    let core = "src/pigeon-core/src/change/lifecycle.rs";
    let approval = "src/pigeon-core/src/approval/grants.rs";
    let deploy = "src/pigeon-core/src/deploy/ship.rs";
    let platform = "src/pigeon-core/src/platform/runs.rs";
    let infra = "src/pigeon-core/src/infra/plans.rs";
    let sites = "src/pigeon-core/src/sites/publish.rs";
    let release = "src/pigeon-core/src/release/lines.rs";
    passes(&layout_with(
        core,
        "pub fn step(c: &Change) -> Change {\n    c.clone()\n}\n",
    ));
    passes(&layout_with(
        core,
        "fn pushed(mut self) -> Self {\n    self\n}\n",
    ));
    for bad in [
        "use std::fs;",
        "let out = std::process::Command::new(\"git\");",
        "use std::net::TcpStream;",
        "let r = ureq::get(url);",
        "worker::console_log!(\"x\");",
        "let at = SystemTime::now();",
        "let at = Instant::now();",
        "fn pushed(&mut self) {}",
        "let j = &mut self.jobs;",
    ] {
        fails(&layout_with(core, bad));
        fails(&layout_with(approval, bad));
        fails(&layout_with(deploy, bad));
        fails(&layout_with(platform, bad));
        fails(&layout_with(infra, bad));
        fails(&layout_with(sites, bad));
        fails(&layout_with(release, bad));
    }
    let shell = "src/pigeon-agent/src/worker/change.rs";
    passes(&layout_with(
        shell,
        "fn apply(&mut self) { std::fs::read(p); }",
    ));
}

#[test]
fn release_changelog_rules() {
    let g = cl_good();
    for content in [
        g.replace("- a v0.1.1\n", "- a 0.1.1\n"),
        g.replace("- a v0.1.1\n", "- a v0.1\n"),
        g.replace("- a v0.1.1\n", "- a/v0.1.1\n"),
        g.replace("### Released\n\n- a v0.1.1\n- b v0.2.0\n", "### Released\n"),
        g.replace("- units renamed · c → b\n", "- units renamed · c\n"),
    ] {
        fails(&changelog(Some(&content)));
    }
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
    let good = "# Each role's capabilities.\nroles:\n  administrator: [read, propose, approve, approve.owner, auto, mandates, platform, members, learner]\n  user: [learner]\n  service: [read, propose]\n";
    let check = |content: &str| {
        violations(
            "roles",
            tree_root(&[(".config/roles.yaml", content)]).path(),
            None,
        )
    };
    passes(&check(good));
    passes(&check(
        &good.replace("user: [learner]", "user: [read, learner]"),
    ));
    passes(&check(
        &good.replace("service: [read, propose]", "service: []"),
    ));
    passes(&violations("roles", tree_root(&[]).path(), None));
    for bad in [
        good.replace(", members, learner]", "]"),
        good.replace("user: [learner]", "user: [*]"),
        good.replace("user: [learner]", "user: [everything]"),
        good.replace("service: [read, propose]", "service: [read, approve]"),
        good.replace("  user: [learner]\n", ""),
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
