//! `pigeon backplane` end to end.
#![cfg(feature = "backplane")]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn backplane() -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.arg("backplane");
    cmd
}

#[test]
fn version_is_backplanes() {
    backplane()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("pigeon-backplane {}\n", backplane::VERSION));
}

#[test]
fn this_instance_is_valid() {
    backplane()
        .args(["instance", "--root"])
        .arg(repo_root())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("instance.show ok"));
}

#[test]
fn engines_and_operations_list() {
    backplane()
        .args(["engines", "--json"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"role\": \"policy\""));
    backplane()
        .arg("operations")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("policy.check (sentinel, read)"));
}

#[test]
fn run_prints_the_json_response() {
    let out = backplane()
        .args(["run", "policy.check", "--json", "--root"])
        .arg(repo_root())
        .assert()
        .code(0);
    let value: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    assert_eq!(value["operation"], "policy.check");
    assert_eq!(value["ok"], true);
}

#[test]
fn an_invalid_manifest_exits_2() {
    let root = TempDir::new().expect("tempdir");
    fs::create_dir_all(root.path().join(".config")).expect("mkdir");
    fs::write(root.path().join(".config/instance.yaml"), "schema: v0\n").expect("write");
    backplane()
        .args(["instance", "--root"])
        .arg(root.path())
        .assert()
        .code(2);
}

#[test]
fn an_unknown_operation_exits_2() {
    backplane()
        .args(["run", "nope.nope", "--root"])
        .arg(repo_root())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no operation `nope.nope`"));
}
