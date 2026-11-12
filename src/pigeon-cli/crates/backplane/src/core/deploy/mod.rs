//! The `deploy` domain: change scope, the ship plan, versions and the test map, as judgements and
//! transitions; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod gate;
#[cfg(feature = "engines")]
pub mod scope;
pub mod ship;
pub mod testmaps;
pub mod versions;

#[cfg(test)]
mod tests;
