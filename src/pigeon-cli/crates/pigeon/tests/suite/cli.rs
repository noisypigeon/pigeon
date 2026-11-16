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
    let listed = |name: &str| {
        out.lines()
            .any(|l| l.split_whitespace().next() == Some(name))
    };
    assert_eq!(listed("eviction"), cfg!(feature = "eviction"));
    for group in ["infra", "sites", "git", "workers", "release"] {
        assert_eq!(listed(group), cfg!(feature = "infra"), "{group}");
    }
    for group in [
        "instance",
        "engines",
        "ops",
        "run",
        "decisions",
        "agent",
        "keys",
    ] {
        assert_eq!(listed(group), cfg!(feature = "agent"), "{group}");
    }
    assert_eq!(listed("signing"), cfg!(feature = "signing"));
    assert_eq!(listed("session"), cfg!(feature = "session"));
    assert!(listed("dev"));
    for old in ["sentinel", "cobot", "backplane"] {
        assert!(!listed(old), "{old} is hidden");
    }
}
