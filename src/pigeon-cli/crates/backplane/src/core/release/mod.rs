//! The `release` domain: release units, changelog tiers, lines and version bumps, from the
//! commits, files and tags the shell reads; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod changelog;
pub mod config;
pub mod lines;
#[cfg(test)]
mod tests;

pub use changelog::Changelog;
pub use config::{CargoTree, Config, Setting, Tier, Unit};
pub use lines::{Commit, Outcome, RELEASE_PREFIX, Released, TagStatus};
