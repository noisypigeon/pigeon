//! `pigeon sentinel check` hands its arguments to `pigeon-sentinel` and returns its exit code;
//! the judge's own behaviour is tested with the binary, in `crates/sentinel/tests/cli.rs`.
#![cfg(feature = "sentinel")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// A stand-in `pigeon-sentinel` in `dir` that prints its arguments and exits with `code`.
fn stand_in(dir: &Path, code: i32) -> PathBuf {
    let p = dir.join("pigeon-sentinel");
    std::fs::write(&p, format!("#!/bin/sh\necho \"args: $*\"\nexit {code}\n")).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    p
}

fn pigeon(sentinel: &Path) -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.env("PIGEON_SENTINEL", sentinel);
    cmd
}

#[test]
fn check_hands_its_arguments_to_pigeon_sentinel() {
    let tmp = TempDir::new().expect("tempdir");
    pigeon(&stand_in(tmp.path(), 0))
        .args([
            "sentinel",
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
fn check_returns_pigeon_sentinels_exit_code() {
    let tmp = TempDir::new().expect("tempdir");
    for code in [1, 2] {
        let dir = tmp.path().join(code.to_string());
        std::fs::create_dir_all(&dir).expect("mkdir");
        pigeon(&stand_in(&dir, code))
            .args(["sentinel", "check"])
            .assert()
            .code(code);
    }
}

#[test]
fn a_missing_pigeon_sentinel_exits_2() {
    pigeon(Path::new("/nonexistent/pigeon-sentinel"))
        .args(["sentinel", "check"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("didn't run"));
}
