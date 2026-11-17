//! Releases: units, scope, restacks and image tool versions. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod git_restack;
#[allow(clippy::module_inception)]
mod release;
mod release_scope;
mod workers_image_tools;
