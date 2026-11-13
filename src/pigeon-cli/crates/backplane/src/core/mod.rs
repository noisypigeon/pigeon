//! Backplane's functional core: types, transitions and judgements, with no I/O, clock or
//! `&mut` receiver; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod app;
pub mod approval;
pub mod change;
pub mod deploy;
#[cfg(feature = "engines")]
pub mod infra;
pub mod insights;
pub mod ledger;
pub mod macos;
pub mod platform;
#[cfg(feature = "engines")]
pub mod release;
pub mod sites;
