//! The binary's top level, whichever features are built.

use assert_cmd::Command;
use predicates::prelude::*;

fn pigeon() -> Command {
    Command::cargo_bin("pigeon").expect("binary")
}

#[test]
fn version_flag() {
    pigeon()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("pigeon 0.1.0"));
}

#[test]
fn only_built_subcommands_exist() {
    let help = pigeon().arg("--help").assert().success();
    let out = String::from_utf8(help.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(out.contains("sentinel"), cfg!(feature = "sentinel"));
    assert_eq!(out.contains("gantry"), cfg!(feature = "gantry"));
    assert_eq!(out.contains("backplane"), cfg!(feature = "backplane"));
}
