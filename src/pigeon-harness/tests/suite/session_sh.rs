//! `tooling/session/session.sh`: the bootstrap's parsing, and the hooks with no `pigeon`; see
//! `.claude/decisions/2026-11-15-development-no-python-rust-tooling.md` §2.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tooling/session/session.sh")
}

/// Runs the script with `args` and `stdin`, a fresh `HOME`, no `pigeon` or `cargo` on `PATH`,
/// and a control plane that doesn't answer.
fn run(args: &[&str], stdin: &str) -> Output {
    let home = tempfile::tempdir().expect("home");
    let mut child = Command::new("sh")
        .arg(script())
        .args(args)
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .env("PIGEON_HOST", "127.0.0.1:9")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sh");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write");
    child.wait_with_output().expect("output")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// `deploy.status`'s answer as recorded on 2026-11-15, trimmed.
const STATUS: &str = r#"{"operation":"deploy.status","ok":true,"result":{"pipeline":{"deployed":"182783cc6e86f799b0582581d291e9d997180c2a","latest_build":{"sentinel_sha256":"b9bd63176c3f2bebc869792446fd2e9736936907c42141ea0456177533de36b1","sha":"d0761620abe1d0d00e8477d2adf94de597d87963","sha256":"5cd1f301e2bdc2efec68ed1e16d3ede20d55ec4201b58893c3ad4108d60504a1"}}}}"#;

#[test]
fn latest_reads_the_build_and_its_sha256() {
    let o = run(&["latest"], STATUS);
    assert!(o.status.success());
    assert_eq!(
        stdout(&o),
        "d0761620abe1d0d00e8477d2adf94de597d87963 5cd1f301e2bdc2efec68ed1e16d3ede20d55ec4201b58893c3ad4108d60504a1\n"
    );
}

#[test]
fn latest_refuses_anything_but_hex_of_the_right_length() {
    for body in [
        r#"{"result":{"pipeline":{"latest_build":{"sha":"../../x","sha256":"00"}}}}"#,
        r#"{"result":{"pipeline":{"latest_build":null}}}"#,
        "upstream request failed",
        "",
    ] {
        let o = run(&["latest"], body);
        assert!(!o.status.success(), "{body}");
        assert_eq!(stdout(&o), "", "{body}");
    }
}

#[test]
fn hooks_degrade_and_exit_0_with_no_pigeon() {
    let brief = run(&["brief"], "");
    assert!(brief.status.success());
    assert!(stdout(&brief).starts_with("pigeon: no pigeon binary yet"));
    let line = run(&["statusline"], "{}");
    assert!(line.status.success());
    assert_eq!(stdout(&line), "pigeon: no pigeon binary yet\n");
    let stop = run(&["stop"], "{}");
    assert!(stop.status.success());
    assert_eq!(stdout(&stop), "");
}

#[test]
fn other_commands_fail_with_no_pigeon_and_no_cargo() {
    let o = run(&["executor", "b1"], "");
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("no cargo"));
}
