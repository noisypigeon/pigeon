//! The instance: manifest, operations, architecture and decisions. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod architecture;
mod decisions;
#[allow(clippy::module_inception)]
mod instance;
