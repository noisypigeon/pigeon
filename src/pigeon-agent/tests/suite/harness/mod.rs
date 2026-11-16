//! Helpers shared by the suite's modules.

#[allow(dead_code)]
pub mod http;
pub mod tools;
#[macro_use]
pub mod kept;
pub mod step;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The judge, `pigeon-eviction` or its old name, built once from this workspace into the tests' own target directory, for
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
        ["pigeon-eviction", "pigeon-sentinel"]
            .into_iter()
            .map(|n| target.join("debug").join(n))
            .find(|p| p.is_file())
            .expect("the judge is built")
    })
}

/// A run's variables: the process's, with `PIGEON_EVICTION` naming [`sentinel`].
#[allow(dead_code)]
pub fn var(k: &str) -> Option<String> {
    if k == "PIGEON_EVICTION" {
        return Some(sentinel().display().to_string());
    }
    std::env::var(k).ok()
}
