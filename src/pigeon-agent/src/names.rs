//! The neutral names, from core; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §5.

pub use pigeon_core::names::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_work_directory_is_git_ignored() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|p| p.join(".config/approvals.yaml").is_file())
            .expect("the repository root");
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect(".gitignore");
        let dir = format!("/{WORK_DIR}/");
        assert!(ignore.lines().any(|l| l == dir), "{dir}");
    }
}
