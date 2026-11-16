//! `pigeon instance`, `engines`, `ops`, `run`, `agent` and `keys` end to end.
#![cfg(feature = "agent")]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

use crate::harness as common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pigeon() -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.env("PIGEON_EVICTION", common::sentinel());
    cmd
}

#[test]
fn version_is_backplanes() {
    pigeon()
        .args(["agent", "--version"])
        .assert()
        .success()
        .stdout(format!("pigeon-agent {}\n", backplane::VERSION));
}

#[test]
fn this_instance_is_valid() {
    pigeon()
        .args(["instance", "--root"])
        .arg(repo_root())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("instance.show ok"));
}

#[test]
fn engines_and_operations_list() {
    pigeon()
        .args(["engines", "--json"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"role\": \"policy\""));
    pigeon()
        .arg("ops")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("policy.check (sentinel, read)"));
}

#[test]
fn run_prints_the_json_response() {
    let out = pigeon()
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
    pigeon()
        .args(["instance", "--root"])
        .arg(root.path())
        .assert()
        .code(2);
}

#[test]
fn an_unknown_operation_exits_2() {
    pigeon()
        .args(["run", "nope.nope", "--root"])
        .arg(repo_root())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no operation `nope.nope`"));
}

// See `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.
#[test]
fn enroll_secret_prints_the_stored_form_and_keeps_the_token_off_stdout() {
    let out = pigeon().args(["keys", "enroll"]).assert().success();
    let out = out.get_output();
    let stdout = String::from_utf8(out.stdout.clone()).expect("UTF-8");
    let stderr = String::from_utf8(out.stderr.clone()).expect("UTF-8");
    let is_hex =
        |s: &str| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let line = stdout.trim_end();
    let (expires, hash) = line.split_once('.').expect("two parts");
    let expires: u64 = expires.parse().expect("an expiry");
    assert!(is_hex(hash), "{hash}");
    let token = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .expect("a token");
    assert!(is_hex(token), "{token}");
    assert_eq!(
        backplane::core::approval::enroll::stored(token, expires),
        line
    );
    assert!(!stdout.contains(token), "the token stays off stdout");
}
