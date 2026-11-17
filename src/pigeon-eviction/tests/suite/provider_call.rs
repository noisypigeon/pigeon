//! The `provider-call` target: a call document judged by every policy whose `when` holds; see
//! `.claude/decisions/2026-11-16-security-provider-calls-mcp-broker.md`'s 2026-11-17 step 5b amendment.

use eviction::{Call, Policy, PullRequest};
use tempfile::TempDir;

const MIRROR: &str = r#"description: A write through the github server names only this repository.
target: provider-call
when:
  - field: { name: server, equals: github }
  - field: { name: effect, equals: write }
rules:
  - field: { name: arguments, matches: '"repo":"noisypigeon/pigeon"' }
"#;

fn call(server: &str, effect: &str, arguments: &str) -> Call {
    [
        ("server", server),
        ("tool", "push_files"),
        ("effect", effect),
        ("caller", "session"),
        ("arguments", arguments),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn mirror() -> Vec<eviction::LoadedPolicy> {
    vec![eviction::load_str("mirror", MIRROR).expect("valid")]
}

#[test]
fn a_write_naming_the_repository_passes_and_another_fails_without_quoting_it() {
    let ok = eviction::judge_call(
        &mirror(),
        &call("github", "write", r#"{"repo":"noisypigeon/pigeon"}"#),
    );
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].violations.len(), 0);
    let bad = eviction::judge_call(
        &mirror(),
        &call("github", "write", r#"{"repo":"someone/else"}"#),
    );
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].violations.len(), 1);
    assert!(!bad[0].violations[0].contains("someone/else"));
}

#[test]
fn a_policy_whose_when_fails_is_left_out() {
    let args = r#"{"repo":"someone/else"}"#;
    assert_eq!(
        eviction::judge_call(&mirror(), &call("github", "read", args)).len(),
        0
    );
    assert_eq!(
        eviction::judge_call(&mirror(), &call("other", "write", args)).len(),
        0
    );
}

#[test]
fn a_missing_field_is_a_violation() {
    let policy = eviction::load_str(
        "needs-change",
        "description: x.\ntarget: provider-call\nrules:\n  - field: { name: change, matches: '^[0-9]+$' }\n",
    )
    .expect("valid");
    let verdicts = eviction::judge_call(&[policy], &call("github", "write", "{}"));
    assert_eq!(verdicts.len(), 1);
    assert!(
        verdicts[0]
            .violations
            .iter()
            .any(|m| m.contains("`change` is not set"))
    );
}

#[test]
fn a_forbidden_pattern_in_arguments_is_not_quoted() {
    let policy = eviction::load_str(
        "no-secret",
        "description: x.\ntarget: provider-call\nrules:\n  - field: { name: arguments, not: 'hunter2' }\n",
    )
    .expect("valid");
    let verdicts = eviction::judge_call(&[policy], &call("github", "write", "pw hunter2 here"));
    assert_eq!(verdicts[0].violations.len(), 1);
    assert!(!verdicts[0].violations[0].contains("pw hunter2"));
}

#[test]
fn invalid_policies_are_refused_with_the_decisions_message() {
    let bad = [
        (
            "entries on provider-call",
            "description: x.\ntarget: provider-call\nrules:\n  - entries: { path: . }\n",
            "only `field` applies to `provider-call`",
        ),
        (
            "at_least on a call field",
            "description: x.\ntarget: provider-call\nrules:\n  - field: { name: tool, at_least: 1 }\n",
            "takes `equals`, or `matches` and/or `not`",
        ),
        (
            "when on tree",
            "description: x.\ntarget: tree\nwhen:\n  - field: { name: server, equals: a }\nrules: []\n",
            "when needs the provider-call target",
        ),
        (
            "server on pull-request",
            "description: x.\ntarget: pull-request\nrules:\n  - field: { name: server, equals: a }\n",
            "the call document's fields need the `provider-call` target",
        ),
    ];
    for (name, yaml, message) in bad {
        let err = Policy::parse(yaml).expect_err(name);
        assert!(err.contains(message), "{name}: {err}");
    }
}

#[test]
fn load_str_names_the_policy_as_given() {
    assert_eq!(
        eviction::load_str("given", MIRROR).expect("valid").name,
        "given"
    );
}

#[test]
fn check_skips_provider_call_policies() {
    let root = TempDir::new().expect("tempdir");
    let pr = PullRequest::default();
    let report = eviction::check(&mirror(), root.path(), Some(&pr)).expect("runs");
    assert_eq!((report.ran, report.skipped), (0, 1));
}

#[test]
fn describe_shows_the_provider_call_kind() {
    let shown = eviction::describe(&mirror());
    assert_eq!(shown[0]["target"]["kind"], "provider-call");
}
