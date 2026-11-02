//! `pigeon sentinel check` end to end.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn check(root: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.args(["sentinel", "check", "--root"]).arg(root);
    for var in ["PR_TITLE", "PR_BODY", "PR_NUMBER"] {
        cmd.env_remove(var);
    }
    cmd
}

fn fixture(policy: &str, files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    fs::create_dir_all(root.path().join(".config/policies")).expect("mkdir");
    fs::write(root.path().join(".config/policies/p.yaml"), policy).expect("write");
    for (name, content) in files {
        fs::write(root.path().join(name), content).expect("write");
    }
    root
}

const POLICY: &str = "description: f starts with ok.\ntarget: { file: f.md }\nrules:\n  - line: { at: 1, matches: '^ok$' }\n";

#[test]
fn this_repository_holds() {
    check(&repo_root())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("policies hold, 1 skipped"));
}

#[test]
fn violation_exits_1_with_path_and_line() {
    let root = fixture(POLICY, &[("f.md", "no\n")]);
    check(root.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "f.md:1: p: line does not match `^ok$`",
        ));
}

#[test]
fn clean_exits_0() {
    let root = fixture(POLICY, &[("f.md", "ok\n")]);
    check(root.path()).assert().code(0);
}

#[test]
fn invalid_policy_exits_2_naming_the_file() {
    let root = fixture(
        "description: x\ntarget: { file: f.md }\nrules:\n  - nope: {}\n",
        &[],
    );
    check(root.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("p.yaml"));
}

#[test]
fn missing_policy_dir_exits_2() {
    let root = TempDir::new().expect("tempdir");
    check(root.path()).assert().code(2);
}

#[test]
fn pull_request_reads_the_environment() {
    let policy = "description: Titles say hi.\ntarget: pull-request\nrules:\n  - field: { name: title, matches: '^hi$' }\n";
    let root = fixture(policy, &[]);
    check(root.path())
        .arg("--pull-request")
        .env("PR_TITLE", "hi")
        .env("PR_BODY", "")
        .env("PR_NUMBER", "1")
        .assert()
        .code(0);
    check(root.path())
        .arg("--pull-request")
        .env("PR_TITLE", "bye")
        .env("PR_BODY", "")
        .env("PR_NUMBER", "1")
        .assert()
        .code(1);
    check(root.path())
        .arg("--pull-request")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("`title` is not set"));
    check(root.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("1 skipped"));
}

#[test]
fn version_flag() {
    Command::cargo_bin("pigeon")
        .expect("binary")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("pigeon 0.1.0"));
}
