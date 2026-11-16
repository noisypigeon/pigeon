//! pigeon's site templates, embedded; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §1.

use include_dir::{Dir, include_dir};

/// The `architecture` template: one diagram of the platform.
pub static ARCHITECTURE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/architecture");
/// The `blog` template.
pub static BLOG: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/blog");

/// Every template, by name, sorted.
#[must_use]
pub fn all() -> [(&'static str, &'static Dir<'static>); 2] {
    [("architecture", &ARCHITECTURE), ("blog", &BLOG)]
}

/// The template named `name`.
#[must_use]
pub fn get(name: &str) -> Option<&'static Dir<'static>> {
    all().into_iter().find(|(n, _)| *n == name).map(|(_, d)| d)
}

/// The text of `path` in the template named `name`.
#[must_use]
pub fn text(name: &str, path: &str) -> Option<&'static str> {
    get(name)?.get_file(path)?.contents_utf8()
}
