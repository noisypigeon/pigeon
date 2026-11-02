//! `pigeon gantry` end to end.
#![cfg(feature = "gantry")]

use assert_cmd::Command;

#[test]
fn reports_not_implemented_and_succeeds() {
    Command::cargo_bin("pigeon")
        .expect("binary")
        .arg("gantry")
        .assert()
        .code(0)
        .stdout(format!("gantry {}: not implemented yet\n", gantry::VERSION));
}
