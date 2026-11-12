//! The `infra` domain: config units, their validation and rendering, from what the shell reads;
//! see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod config;
pub mod engines;
pub mod manifest;
pub mod moves;
pub mod plans;
pub mod providers;
pub mod unit;
