//! An executor's printed arguments, `config.yaml`'s repository, and main's build, without a
//! network; ported from `tooling/session/test_session.py`.

use serde_json::{Value, json};

use super::{Executor, executor, latest_build, repository, sha256, verified};

const PLANNER: &str = "https://claude.ai/code/session_PLANNER";
const ADR: &str = ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md";
const REPO: &str = "https://github.com/noisypigeon/pigeon";

fn brief(model: &str, adr: Option<&str>) -> Value {
    json!({
        "id": "b7",
        "planner": PLANNER,
        "revisions": [
            {"draft": {"model": model, "title": "Old"}},
            {"draft": {"model": model, "title": "A step", "adr": adr}},
        ],
    })
}

fn models() -> Value {
    json!({"haiku": "claude-haiku-5-5", "sonnet": "claude-sonnet-5-5", "opus": "claude-opus-5-5"})
}

fn call(b: &Value, extra: &[&str]) -> Result<Executor, String> {
    let extra: Vec<String> = extra.iter().map(ToString::to_string).collect();
    executor("b7", b, &models(), REPO, &extra)
}

#[test]
fn it_prints_every_argument_create_session_needs() {
    let got = call(&brief("sonnet", Some(ADR)), &[]).expect("arguments");
    let printed: Value = serde_json::to_value(&got).expect("json");
    let keys: Vec<&String> = printed.as_object().expect("object").keys().collect();
    assert_eq!(keys.len(), 5);
    assert_eq!(got.model, "claude-sonnet-5-5");
    assert_eq!(got.title, "pigeon b7 (sonnet): A step");
    assert_eq!(got.source_url, REPO);
    assert_eq!(
        got.tags,
        [
            "pigeon:brief-b7",
            "pigeon:adr-2026-11-13-development-model-tiered-agent-briefs"
        ]
    );
}

#[test]
fn it_prints_keys_in_session_pys_order_indented_by_two() {
    let got = call(&brief("sonnet", Some(ADR)), &[]).expect("arguments");
    let text = serde_json::to_string_pretty(&got).expect("json");
    let at = |k: &str| text.find(&format!("\n  \"{k}\": ")).expect(k);
    assert!(at("model") < at("title"));
    assert!(at("title") < at("prompt"));
    assert!(at("prompt") < at("source_url"));
    assert!(at("source_url") < at("tags"));
}

#[test]
fn the_prompt_checks_the_checkout_and_tools_before_anything() {
    let prompt = call(&brief("sonnet", Some(ADR)), &[])
        .expect("arguments")
        .prompt;
    let guard = prompt.find("0. Check first").expect("guard");
    assert!(guard < prompt.find("1. Per CLAUDE.md").expect("step 1"));
    assert!(prompt.contains("`send_message` your planner's session, `session_PLANNER`"));
    assert!(prompt.contains("Never `add_repo`"));
    assert!(prompt.contains("revision 2"));
    assert!(prompt.contains(&format!("`planner` \"{PLANNER}\"")));
    assert!(prompt.contains("arm `Monitor` with `sh tooling/session/session.sh follow`"));
    assert!(!prompt.contains("python"));
    for hole in [
        "{model}",
        "{brief}",
        "{revision}",
        "{planner}",
        "{planner_session}",
    ] {
        assert!(!prompt.contains(hole), "{hole}");
    }
}

#[test]
fn tags_add_each_tag_once() {
    let got = call(
        &brief("sonnet", Some(ADR)),
        &[
            "pigeon:reliability",
            "pigeon:brief-b7",
            "pigeon:reliability",
        ],
    )
    .expect("arguments");
    assert_eq!(
        got.tags.last().map(String::as_str),
        Some("pigeon:reliability")
    );
    assert_eq!(
        got.tags.iter().filter(|t| *t == "pigeon:brief-b7").count(),
        1
    );
    assert_eq!(
        got.tags
            .iter()
            .filter(|t| *t == "pigeon:reliability")
            .count(),
        1
    );
}

#[test]
fn a_brief_without_an_adr_still_has_its_brief_tag() {
    let got = call(&brief("sonnet", None), &[]).expect("arguments");
    assert_eq!(got.tags, ["pigeon:brief-b7"]);
}

#[test]
fn no_repository_means_none() {
    assert_eq!(repository("name: x\n"), None);
    assert_eq!(repository("repository: ''\n"), None);
    assert_eq!(
        repository("name: x\nrepository: 'https://e.org/r'\n").as_deref(),
        Some("https://e.org/r")
    );
}

#[test]
fn refusals() {
    assert_eq!(call(&Value::Null, &[]), Err("no brief `b7`".into()));
    assert_eq!(
        call(&json!({"id": "b7", "revisions": []}), &[]),
        Err("no brief `b7`".into())
    );
    assert_eq!(
        call(&brief("opus", Some(ADR)), &[]),
        Err("brief b7 is for `opus`, which no executor carries out".into())
    );
    assert!(call(&brief("haiku", None), &[]).is_ok());
}

/// `deploy.status`'s answer as recorded on 2026-11-15, trimmed.
const STATUS: &str = r#"{"pipeline":{"deployed":"182783cc6e86f799b0582581d291e9d997180c2a","latest_build":{"sentinel_sha256":"b9bd63176c3f2bebc869792446fd2e9736936907c42141ea0456177533de36b1","sha":"d0761620abe1d0d00e8477d2adf94de597d87963","sha256":"5cd1f301e2bdc2efec68ed1e16d3ede20d55ec4201b58893c3ad4108d60504a1"}}}"#;

#[test]
fn latest_build_reads_hex_only() {
    let v: Value = serde_json::from_str(STATUS).expect("json");
    assert_eq!(
        latest_build(&v),
        Some((
            "d0761620abe1d0d00e8477d2adf94de597d87963".into(),
            "5cd1f301e2bdc2efec68ed1e16d3ede20d55ec4201b58893c3ad4108d60504a1".into()
        ))
    );
    let bad = json!({"pipeline": {"latest_build": {"sha": "../x", "sha256": "00"}}});
    assert_eq!(latest_build(&bad), None);
    assert_eq!(latest_build(&json!({})), None);
}

#[test]
fn verified_checks_the_sha256() {
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(sha256(b"abc"), abc);
    assert!(verified(b"abc", abc));
    assert!(verified(b"abc", &abc.to_uppercase()));
    assert!(!verified(b"abd", abc));
}
