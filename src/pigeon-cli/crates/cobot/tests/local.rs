//! Local units run only when named; see
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn unit(root: &Path, rel: &str, local: Option<Value>) {
    let mut x = json!({ "version": "0.1.0", "secrets": [], "providers": {} });
    if let Some(l) = local {
        x["local"] = l;
    }
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": { "name": { "type": "string" } },
        "x-cobot": x
    });
    write(root, &format!("{rel}/schema.json"), &schema.to_string());
    write(root, &format!("{rel}/config.yaml"), "name: unit\n");
}

fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    unit(dir.path(), "infra/shared", None);
    unit(dir.path(), "infra/boot", Some(Value::Bool(true)));
    dir
}

fn paths(units: &[cobot::Unit]) -> Vec<&str> {
    units.iter().map(|u| u.path.as_str()).collect()
}

#[test]
fn a_local_unit_is_validated_but_skipped_unless_named() {
    let repo = repo();
    let root = repo.path();
    let providers = root.join(".config");
    let all = cobot::validate(root, &providers).expect("valid");
    assert_eq!(paths(&all), ["infra/boot", "infra/shared"]);
    let unnamed = cobot::select(root, &providers, &[]).expect("select");
    assert_eq!(paths(&unnamed), ["infra/shared"]);
    let named = cobot::select(root, &providers, &["infra/boot".into()]).expect("select");
    assert_eq!(paths(&named), ["infra/boot"]);
    assert!(named[0].is_local());
    assert!(!unnamed[0].is_local());
}

#[test]
fn local_false_runs_like_any_unit() {
    let dir = TempDir::new().expect("tempdir");
    unit(dir.path(), "infra/plain", Some(Value::Bool(false)));
    let selected = cobot::select(dir.path(), &dir.path().join(".config"), &[]).expect("select");
    assert_eq!(paths(&selected), ["infra/plain"]);
}

#[test]
fn a_non_boolean_local_fails() {
    for bad in [json!("yes"), json!(1), json!(null)] {
        let dir = TempDir::new().expect("tempdir");
        unit(dir.path(), "infra/boot", Some(bad.clone()));
        let problems = cobot::unit::validate(dir.path(), &cobot::providers::Allowlist::default());
        assert!(
            problems.iter().any(
                |p| p.path == "infra/boot/schema.json" && p.message.contains("`x-cobot.local`")
            ),
            "{bad}: {problems:?}"
        );
    }
}
