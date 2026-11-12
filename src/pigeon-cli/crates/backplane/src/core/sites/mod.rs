//! The `sites` domain: a site's layout, its assembled project, what it publishes and its live
//! checks, from what the shell reads; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

#[cfg(feature = "engines")]
pub mod assemble;
#[cfg(feature = "engines")]
pub mod checks;
#[cfg(feature = "engines")]
pub mod layout;
#[cfg(feature = "engines")]
pub mod publish;
pub mod view;

#[cfg(all(test, feature = "engines"))]
mod tests;
