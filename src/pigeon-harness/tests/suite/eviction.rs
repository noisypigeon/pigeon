//! `pigeon eviction check` hands its arguments to `pigeon-eviction` and returns its exit code;
//! the judge's own behaviour is tested with the binary, in `src/pigeon-eviction/tests/suite/cli.rs`.
#![cfg(feature = "eviction")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// A stand-in `pigeon-eviction` in `dir` that prints its arguments and exits with `code`.
fn stand_in(dir: &Path, code: i32) -> PathBuf {
    let p = dir.join("pigeon-eviction");
    std::fs::write(&p, format!("#!/bin/sh\necho \"args: $*\"\nexit {code}\n")).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    p
}

fn pigeon(judge: &Path) -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.env("PIGEON_EVICTION", judge);
    cmd
}

#[test]
fn check_hands_its_arguments_to_the_judge() {
    let tmp = TempDir::new().expect("tempdir");
    pigeon(&stand_in(tmp.path(), 0))
        .args([
            "eviction",
            "check",
            "--root",
            "r",
            "--policies",
            "p",
            "--pull-request",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "args: check --root r --policies p --pull-request",
        ));
}

#[test]
fn check_returns_the_judges_exit_code() {
    let tmp = TempDir::new().expect("tempdir");
    for code in [1, 2] {
        let dir = tmp.path().join(code.to_string());
        std::fs::create_dir_all(&dir).expect("mkdir");
        pigeon(&stand_in(&dir, code))
            .args(["eviction", "check"])
            .assert()
            .code(code);
    }
}

#[test]
fn a_missing_judge_exits_2() {
    pigeon(Path::new("/nonexistent/pigeon-eviction"))
        .args(["eviction", "check"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("didn't run"));
}
