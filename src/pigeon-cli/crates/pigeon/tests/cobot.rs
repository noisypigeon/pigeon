//! `pigeon cobot` end to end.
#![cfg(feature = "cobot")]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cobot() -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.arg("cobot").env_remove("COBOT_STATE_PASSPHRASE");
    cmd
}

#[test]
fn validate_passes_on_an_empty_repository() {
    let root = TempDir::new().expect("tempdir");
    cobot()
        .args(["validate", "--root"])
        .arg(root.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 unit(s) valid"));
}

#[test]
fn validate_fails_with_exit_1() {
    let root = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(root.path().join("infra/u")).expect("mkdir");
    std::fs::write(
        root.path().join("infra/u/unit.yaml"),
        "schema: 0\nname: u\nkind: workload\nmodules: []\n",
    )
    .expect("write");
    cobot()
        .args(["validate", "--root"])
        .arg(root.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("kind does not match"));
}

#[test]
fn plan_refuses_without_a_passphrase() {
    let root = TempDir::new().expect("tempdir");
    cobot()
        .args(["plan", "--root"])
        .arg(root.path())
        .args(["--state"])
        .arg(root.path().join("state"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "COBOT_STATE_PASSPHRASE must be set",
        ));
}

#[test]
fn cobot_reports_its_own_version() {
    cobot()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(cobot::VERSION));
}
