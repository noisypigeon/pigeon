//! The `change` target: a change's branch, scope and size; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §3.

use eviction::{LoadedPolicy, Policy, PullRequest};
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
        ..PullRequest::default()
    }
}

fn check(pr: Option<&PullRequest>) -> (usize, Vec<String>) {
    let root = TempDir::new().expect("tempdir");
    let report = eviction::check(&[loaded(POLICY).expect("valid")], root.path(), pr).expect("runs");
    (
        report.skipped,
        report.violations.iter().map(ToString::to_string).collect(),
    )
}

#[test]
fn a_change_inside_its_scope_passes() {
    let pr = change(
        Some("change/3-eviction-change-target"),
        "src/pigeon-eviction/src/lib.rs\n.claude/decisions/a.md\nCLAUDE.md",
        "src/pigeon-eviction/**\n.claude/decisions/\nCLAUDE.md",
    );
    let (skipped, got) = check(Some(&pr));
    assert_eq!(skipped, 0);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn a_path_outside_the_scope_is_named() {
    let pr = change(
        Some("change/3-x"),
        "src/pigeon-agent/src/git/mod.rs\n.config/policies/layout.yaml",
        "src/pigeon-agent/**",
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

const TESTS_POLICY: &str = r"description: A change that adds a test says why in its first commit's Test-Reason trailer; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §C.2.
target: change
when:
  - field:
      name: added_tests
      at_least: 1
rules:
  - field:
      name: test_reason
      matches: '(?s)\s*\S.*'
";

const ENUMERATIONS_POLICY: &str = r"description: 'A test that pins a per-feature count or lists catalog names is refused unless its Test-Reason starts with enumeration: and says why; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §C.2.'
target: change
when:
  - field:
      name: enumerations
      at_least: 1
rules:
  - field:
      name: test_reason
      matches: '(?s)\s*enumeration:\s*\S.*'
";

fn inline_policy(name: &str) -> LoadedPolicy {
    let text = if name == "tests.yaml" {
        TESTS_POLICY
    } else {
        ENUMERATIONS_POLICY
    };
    loaded(text).expect("valid")
}

fn tests_verdict(
    name: &str,
    added_tests: Option<&str>,
    enumerations: Option<&str>,
    reason: Option<&str>,
) -> Vec<String> {
    let pr = PullRequest {
        added_tests: added_tests.map(Into::into),
        enumerations: enumerations.map(Into::into),
        test_reason: reason.map(Into::into),
        ..change(Some("change/3-x"), "a", "a")
    };
    let root = TempDir::new().expect("tempdir");
    eviction::check(&[inline_policy(name)], root.path(), Some(&pr))
        .expect("runs")
        .violations
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn added_tests_without_a_reason_are_refused() {
    assert_ne!(
        tests_verdict("tests.yaml", Some("2"), None, Some("")),
        Vec::<String>::new()
    );
}

#[test]
fn added_tests_with_a_reason_pass() {
    let got = tests_verdict(
        "tests.yaml",
        Some("2"),
        None,
        Some("guards the lease fence"),
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn no_added_tests_need_no_reason() {
    assert_eq!(
        tests_verdict("tests.yaml", Some("0"), None, None),
        Vec::<String>::new()
    );
}

#[test]
fn an_uncomputed_count_does_not_apply_the_tests_policy() {
    assert_eq!(
        tests_verdict("tests.yaml", None, None, None),
        Vec::<String>::new()
    );
}

#[test]
fn an_enumeration_needs_its_own_reason() {
    let name = "tests-enumerations.yaml";
    assert_ne!(
        tests_verdict(name, None, Some("1"), Some("guards x")),
        Vec::<String>::new()
    );
    let got = tests_verdict(
        name,
        None,
        Some("1"),
        Some("enumeration: the closed set of view kinds"),
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn when_and_the_test_fields_are_limited_to_their_targets() {
    let tree =
        "description: d\ntarget: tree\nwhen:\n  - field: { name: title, matches: x }\nrules: []\n";
    assert!(
        loaded(tree)
            .unwrap_err()
            .contains("when needs the provider-call or change target")
    );
    let pull = "description: d\ntarget: pull-request\nrules:\n  - field: { name: test_reason, matches: x }\n";
    assert!(
        loaded(pull)
            .unwrap_err()
            .contains("need the `change` target")
    );
}
