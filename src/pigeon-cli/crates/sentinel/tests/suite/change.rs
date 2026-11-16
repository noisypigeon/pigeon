//! The `change` target: a change's branch, scope and size; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §3.

use sentinel::{LoadedPolicy, Policy, PullRequest};
use tempfile::TempDir;

const POLICY: &str = r"description: A change stays on its branch and inside its scope.
target: change
rules:
  - field:
      name: branch
      matches: '^change/[1-9][0-9]*-[a-z0-9]+(-[a-z0-9]+)*$'
  - field:
      name: files
      within: scope
  - field:
      name: files
      max_lines: 3
";

fn loaded(yaml: &str) -> Result<LoadedPolicy, String> {
    Policy::parse(yaml).map(|policy| LoadedPolicy {
        name: "change".into(),
        policy,
    })
}

fn change(branch: Option<&str>, files: &str, scope: &str) -> PullRequest {
    PullRequest {
        title: Some("feat(x): y".into()),
        body: Some("body".into()),
        number: Some("3".into()),
        files: Some(files.into()),
        branch: branch.map(Into::into),
        scope: Some(scope.into()),
    }
}

fn check(pr: Option<&PullRequest>) -> (usize, Vec<String>) {
    let root = TempDir::new().expect("tempdir");
    let report = sentinel::check(&[loaded(POLICY).expect("valid")], root.path(), pr).expect("runs");
    (
        report.skipped,
        report.violations.iter().map(ToString::to_string).collect(),
    )
}

#[test]
fn a_change_inside_its_scope_passes() {
    let pr = change(
        Some("change/3-sentinel-change-target"),
        "src/pigeon-cli/crates/sentinel/src/lib.rs\n.claude/decisions/a.md\nCLAUDE.md",
        "src/pigeon-cli/crates/sentinel/**\n.claude/decisions/\nCLAUDE.md",
    );
    let (skipped, got) = check(Some(&pr));
    assert_eq!(skipped, 0);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_path_outside_the_scope_is_named() {
    let pr = change(
        Some("change/3-x"),
        "src/pigeon-cli/crates/cobot/src/git.rs\n.config/policies/layout.yaml",
        "src/pigeon-cli/crates/cobot/**",
    );
    let got = check(Some(&pr)).1;
    assert!(
        got.iter()
            .any(|v| v.contains("outside `scope`: .config/policies/layout.yaml")),
        "{got:?}"
    );
    assert!(!got.iter().any(|v| v.contains("git.rs")), "{got:?}");
}

#[test]
fn a_star_stays_within_one_directory() {
    let pr = change(Some("change/3-x"), "a/b/c.rs", "a/*.rs");
    assert_eq!(check(Some(&pr)).1.len(), 1, "`*` stops at `/`");
    let pr = change(Some("change/3-x"), "a/c.rs", "a/*.rs");
    assert_eq!(check(Some(&pr)).1, Vec::<String>::new());
}

#[test]
fn a_bad_branch_and_too_many_files_are_refused() {
    let pr = change(Some("main"), "a\nb\nc\nd", "**");
    let got = check(Some(&pr)).1;
    assert!(
        got.iter()
            .any(|v| v.contains("`branch` `main` does not match")),
        "{got:?}"
    );
    assert!(
        got.iter()
            .any(|v| v.contains("`files` has 4 lines; at most 3")),
        "{got:?}"
    );
}

#[test]
fn a_pull_request_without_a_branch_skips_the_change_policy() {
    let pr = change(None, "a", "a");
    assert_eq!(check(Some(&pr)).0, 1);
    assert_eq!(check(None).0, 1);
}

#[test]
fn change_fields_need_the_change_target_and_one_kind_per_rule() {
    let pull =
        "description: d\ntarget: pull-request\nrules:\n  - field: { name: branch, matches: x }\n";
    assert!(
        loaded(pull)
            .unwrap_err()
            .contains("need the `change` target")
    );
    let two = "description: d\ntarget: change\nrules:\n  - field: { name: files, within: scope, max_lines: 2 }\n";
    assert!(loaded(two).unwrap_err().contains("takes one of"));
}
