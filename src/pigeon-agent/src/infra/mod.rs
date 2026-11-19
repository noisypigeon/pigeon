//! Infrastructure as code: units, tofu plans and applies, state and secrets; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

pub mod adopt;
pub mod catalog;
pub mod config;
mod engines;
pub mod generations;
pub mod manifest;
pub mod moves;
pub mod plans;
pub mod providers;
pub mod s3;
pub mod secrets;
pub mod session;
pub mod state;
mod tofu;
pub mod unit;
pub mod zone_import;

pub use engines::*;
