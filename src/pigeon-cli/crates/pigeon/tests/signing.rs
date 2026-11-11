//! `pigeon backplane signing`, the parts that run off macOS.
#![cfg(feature = "signing")]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn signing(tmp: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.args(["backplane", "signing"])
        .env("TMPDIR", tmp.path())
        .env("BUILDKITE_JOB_ID", "0190-abcd")
        .env("HOME", tmp.path())
        .env_remove("BUILDKITE_STEP_KEY")
        // Nothing is reachable, so a load that got past its guard would fail differently.
        .env("PATH", tmp.path());
    cmd
}

#[test]
fn load_outside_the_sign_step_stops_before_any_request() {
    let tmp = TempDir::new().unwrap();
    signing(&tmp)
        .args(["load", "--platform", "http://127.0.0.1:9"])
        .env("BUILDKITE_STEP_KEY", "archive")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "runs only in step `sign`, not `archive`",
        ));
    signing(&tmp)
        .arg("load")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("BUILDKITE_STEP_KEY is unset"));
    assert!(!tmp.path().join("pigeon-signing-0190-abcd").exists());
}

#[test]
fn load_without_an_oidc_token_is_refused_and_leaves_nothing() {
    let tmp = TempDir::new().unwrap();
    signing(&tmp)
        .arg("load")
        .env("BUILDKITE_STEP_KEY", "sign")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no OIDC token"));
    assert!(!tmp.path().join("pigeon-signing-0190-abcd").exists());
}

#[test]
fn clean_with_nothing_loaded_succeeds() {
    let tmp = TempDir::new().unwrap();
    signing(&tmp).arg("clean").assert().success();
}

#[test]
fn clean_removes_a_directory_left_without_state() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("pigeon-signing-0190-abcd");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("in.p12"), b"x").unwrap();
    signing(&tmp).arg("clean").assert().success();
    assert!(!dir.exists());
}

#[test]
fn a_malformed_job_id_is_refused() {
    let tmp = TempDir::new().unwrap();
    signing(&tmp)
        .arg("clean")
        .env("BUILDKITE_JOB_ID", "../escape")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("BUILDKITE_JOB_ID is malformed"));
}

#[test]
fn upload_with_nothing_loaded_says_so() {
    let tmp = TempDir::new().unwrap();
    signing(&tmp)
        .args(["upload", "--ipa", "Pigeon.ipa"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing is loaded"));
}
