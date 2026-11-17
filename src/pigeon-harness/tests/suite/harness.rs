//! Helpers shared by the suite's modules.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The judge, `pigeon-eviction`, built once from this workspace into the tests' own target directory, for
/// the tests that need the real judge; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1.
///
/// # Panics
///
/// When it doesn't build.
#[allow(dead_code)]
pub fn judge() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pigeon-eviction");
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let status = std::process::Command::new(env!("CARGO"))
            .args([
                "build",
                "--quiet",
                "--locked",
                "-p",
                "pigeon-eviction",
                "--bins",
            ])
            .arg("--target-dir")
            .arg(&target)
            .current_dir(&workspace)
            .status()
            .expect("cargo runs");
        assert!(status.success(), "the judge builds");
        target.join("debug").join("pigeon-eviction")
    })
}

/// A run's variables: the process's, with `PIGEON_EVICTION` naming [`judge`].
#[allow(dead_code)]
pub fn var(k: &str) -> Option<String> {
    if k == "PIGEON_EVICTION" {
        return Some(judge().display().to_string());
    }
    std::env::var(k).ok()
}
