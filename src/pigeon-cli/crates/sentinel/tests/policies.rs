//! Runs this repository's real policies against fixtures; each rule must reject bad input.

use std::fs;
use std::path::{Path, PathBuf};

use sentinel::{LoadedPolicy, PullRequest};
use tempfile::TempDir;

const URL: &str = "https://github.com/noisypigeon/pigeon";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const GOOD_NAME: &str = "2026-01-02-development-a-sample-decision.md";

fn policy(name: &str) -> LoadedPolicy {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../.config/policies")
        .join(format!("{name}.yaml"));
    sentinel::load_file(&path).expect("policy loads")
}

fn violations(name: &str, root: &Path, pr: Option<&PullRequest>) -> Vec<String> {
    sentinel::check(&[policy(name)], root, pr)
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
        "\n## Amendments\n\n### 2026-02-01: Later\n\n### 2026-01-05: Earlier\n",
        "\n## Amendments\n\n### 2026-01-01: Too early\n",
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
        Some("src/a.rs\nsrc/pigeon-cli/crates/gantry/CHANGELOG.md"),
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
    let report = sentinel::check(&[policy("pull-request")], pr_root().path(), None).expect("runs");
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
        "# Changelog\n\n## 2026-01-03\n\n- ci: second ([#2]({URL}/pull/2))\n- feat: first ([#1]({URL}/pull/1))\n\n## 2026-01-02\n\n- docs(adr): bootstrap ([0123456]({URL}/commit/{SHA}))\n"
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
    ] {
        fails(&changelog(Some(&content)));
    }
}

#[test]
fn every_policy_is_valid() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../.config/policies");
    assert_eq!(sentinel::load_dir(&dir).expect("loads").len(), 15);
}

fn layout_root(skip: &[&str], extra: &[&str]) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let dirs = [
        ".claude",
        ".config",
        ".github",
        ".git",
        "src/pigeon-cli",
        "tooling",
        "catalog",
        "infra",
        "workloads",
    ];
    let files = [
        ".gitignore",
        "CHANGELOG.md",
        "CLAUDE.md",
        "mise.toml",
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
        (&["src/pigeon-cli"], &[]),
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
fn layout_keeps_backplane_core_pure() {
    let core = "src/pigeon-cli/crates/backplane/src/core/change/lifecycle.rs";
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
    }
    let shell = "src/pigeon-cli/crates/backplane/src/worker/change.rs";
    passes(&layout_with(
        shell,
        "fn apply(&mut self) { std::fs::read(p); }",
    ));
}

fn release_changelog(content: &str) -> Vec<String> {
    let root = TempDir::new().expect("tempdir");
    let path = root
        .path()
        .join("src/pigeon-cli/crates/sentinel/CHANGELOG.md");
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
    violations("release-changelog", root.path(), None)
}

#[test]
fn release_changelog_rules() {
    let g = format!(
        "# Changelog\n\n## v0.1.1 (2026-01-03)\n\n- feat: b ([#2]({URL}/pull/2))\n\n## v0.1.0 (2026-01-03)\n\n- feat: a ([#1]({URL}/pull/1))\n- chore: x ([0123456]({URL}/commit/{SHA}))\n"
    );
    passes(&release_changelog(&g));
    for content in [
        g.replace("v0.1.0 (2026-01-03)", "v0.1.0 (2026-01-04)"),
        g.replace("v0.1.0", "v0.1.1"),
        g.replace("## v0.1.0 (2026-01-03)", "## 2026-01-03"),
        g.replace(&format!("- feat: b ([#2]({URL}/pull/2))\n"), ""),
        g.replace("[#2]", "[#3]"),
    ] {
        fails(&release_changelog(&content));
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

#[test]
fn terraform_rules() {
    let ok = [
        ("infra/a/unit.yaml", ""),
        ("infra/a/modules/m/main.tf", ""),
        ("catalog/c/modules/m/main.tf", ""),
        ("workloads/w/unit.yaml", ""),
        ("src/x/target/debug/build.tf", ""),
        (".cobot/infra/a/main.tf.json", ""),
    ];
    passes(&violations("terraform", tree_root(&ok).path(), None));
    for bad in [
        "src/pigeon-cli/main.tf",
        "tooling/x.tf.json",
        "x.tofu",
        "src/vars.tfvars",
        ".terraform.lock.hcl",
        "src/unit.yaml",
        "catalog/c/unit.yaml",
        "catalog/c/vars.tfvars",
        "catalog/c/.terraform.lock.hcl",
        "catalog/c/secrets.config.yaml",
        "infra/a/terraform.tfstate",
        "workloads/w/terraform.tfstate.backup",
        "infra/a/.terraform/providers/x",
    ] {
        let mut files = ok.to_vec();
        files.push((bad, ""));
        let v = violations("terraform", tree_root(&files).path(), None);
        assert!(v.iter().any(|m| m.starts_with(bad)), "{bad}: {v:?}");
    }
}

#[test]
fn mirror_forbids_github_workflows() {
    let ok = [(".github/pull_request_template.md", "# Summary\n")];
    passes(&violations("mirror", tree_root(&ok).path(), None));
    for file in [".github/workflows/rust.yml", ".github/workflows/x.yaml"] {
        let mut files = ok.to_vec();
        files.push((file, "on: push\n"));
        let v = violations("mirror", tree_root(&files).path(), None);
        assert!(
            v.iter().any(|m| m.starts_with(&format!("{file}:1:"))),
            "{file}: {v:?}"
        );
    }
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
fn migrations_only_expand() {
    let dir = "src/pigeon-cli/crates/backplane/migrations";
    let ok = [
        (
            "src/pigeon-cli/crates/backplane/migrations/0001_schema.sql",
            "CREATE TABLE runs (id TEXT PRIMARY KEY);\n",
        ),
        (
            "src/pigeon-cli/crates/backplane/migrations/0002_more.sql",
            "ALTER TABLE runs ADD COLUMN at INTEGER;\n",
        ),
    ];
    passes(&violations("migrations", tree_root(&ok).path(), None));
    for sql in [
        "DROP TABLE runs;\n",
        "ALTER TABLE runs DROP COLUMN at;\n",
        "ALTER TABLE runs RENAME TO jobs;\n",
        "alter table runs rename column at to started;\n",
    ] {
        let file = format!("{dir}/0003_contract.sql");
        let mut files = ok.to_vec();
        files.push((file.as_str(), sql));
        let v = violations("migrations", tree_root(&files).path(), None);
        assert!(v.iter().any(|m| m.starts_with(&file)), "{sql}: {v:?}");
    }
}

#[test]
fn sentinel_actions_keeps_sentinel_read_only() {
    let ok = [
        (
            "src/pigeon-cli/crates/sentinel/src/lib.rs",
            "use std::fs;\n",
        ),
        (
            "src/pigeon-cli/crates/sentinel/tests/t.rs",
            "fs::write(p, c);\n",
        ),
        (
            "src/pigeon-cli/crates/cobot/src/release/git.rs",
            "Command::new(\"git\");\n",
        ),
    ];
    passes(&violations("sentinel-actions", tree_root(&ok).path(), None));
    for (file, content) in [
        (
            "src/pigeon-cli/crates/sentinel/src/lib.rs",
            "use std::process::Command;\n",
        ),
        (
            "src/pigeon-cli/crates/sentinel/src/x.rs",
            "let c = Command::new(\"git\");\n",
        ),
        (
            "src/pigeon-cli/crates/sentinel/src/y.rs",
            "fs::write(p, c)?;\n",
        ),
    ] {
        let mut files = ok.to_vec();
        files.retain(|(f, _)| *f != file);
        files.push((file, content));
        let v = violations("sentinel-actions", tree_root(&files).path(), None);
        assert!(
            v.iter().any(|m| m.starts_with(&format!("{file}:1:"))),
            "{file}: {v:?}"
        );
    }
}
