//! Runs the built binary and checks its version line.

use std::process::Command;

#[test]
fn prints_its_version_and_sentinels() {
    let output = Command::new(env!("CARGO_BIN_EXE_pigeon"))
        .output()
        .expect("pigeon runs");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf-8 output"),
        format!("pigeon {v} (sentinel {v})\n", v = env!("CARGO_PKG_VERSION"))
    );
}
