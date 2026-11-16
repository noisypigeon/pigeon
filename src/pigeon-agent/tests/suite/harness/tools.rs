//! The tools a module runs, and whether a test may skip without them: never under
//! `PIGEON_REQUIRE_TOOLS`; see `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §4.

use std::process::Command;

/// Whether `tofu` runs.
pub fn tofu_available() -> bool {
    available("tofu", "version")
}

/// Whether `zola` runs.
pub fn zola_available() -> bool {
    available("zola", "--version")
}

/// Whether `tool` answers `arg`; when it doesn't and `PIGEON_REQUIRE_TOOLS` is set, the test fails.
fn available(tool: &str, arg: &str) -> bool {
    let found = Command::new(tool)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        found || std::env::var_os("PIGEON_REQUIRE_TOOLS").is_none(),
        "{tool} is not on PATH, and PIGEON_REQUIRE_TOOLS forbids skipping"
    );
    found
}
