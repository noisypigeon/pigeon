//! The file reads core's checks need, which the shell does; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §2.

use std::path::Path;

/// Reads files for core; agent's implementation reads the disk.
pub trait Files: std::fmt::Debug + Send + Sync {
    /// The file at `path` as text, if it reads.
    fn read(&self, path: &Path) -> Option<String>;
    /// The names of the directories directly under `path`; none if it doesn't list.
    fn dirs(&self, path: &Path) -> Vec<String>;
    /// Whether `path` is a file.
    fn is_file(&self, path: &Path) -> bool;
}
