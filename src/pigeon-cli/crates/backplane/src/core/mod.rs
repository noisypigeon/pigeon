//! Backplane's functional core: types, transitions and judgements, with no I/O, clock or
//! `&mut` receiver; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod account;
pub mod app;
pub mod approval;
pub mod bench;
pub mod brief;
pub mod change;
pub mod connect;
pub mod deploy;
pub mod infra;
pub mod insights;
pub mod learner;
pub mod ledger;
pub mod macos;
pub mod member;
pub mod minting;
pub mod owner;
pub mod platform;
pub mod pool;
pub mod release;
pub mod session;
pub mod sites;
