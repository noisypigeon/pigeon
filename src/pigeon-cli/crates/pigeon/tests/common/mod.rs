//! Helpers shared by the integration tests.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `pigeon-sentinel`, built once from this workspace into the tests' own target directory, for
/// the tests that need the real judge; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1.
///
/// # Panics
///
/// When it doesn't build.
#[allow(dead_code)]
pub fn sentinel() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pigeon-sentinel");
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--locked", "-p", "pigeon-sentinel"])
            .arg("--target-dir")
            .arg(&target)
            .current_dir(&workspace)
            .status()
            .expect("cargo runs");
        assert!(status.success(), "pigeon-sentinel builds");
        target.join("debug").join("pigeon-sentinel")
    })
}

/// A run's variables: the process's, with `PIGEON_SENTINEL` naming [`sentinel`].
#[allow(dead_code)]
pub fn var(k: &str) -> Option<String> {
    if k == "PIGEON_SENTINEL" {
        return Some(sentinel().display().to_string());
    }
    std::env::var(k).ok()
}
