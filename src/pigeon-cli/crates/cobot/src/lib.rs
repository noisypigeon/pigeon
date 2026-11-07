//! Infrastructure as code; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

#[cfg(feature = "engines")]
pub mod apex;
#[cfg(feature = "engines")]
pub mod artifacts;
#[cfg(any(feature = "engines", feature = "worker"))]
pub mod artifacts_wire;
#[cfg(feature = "engines")]
pub mod catalog;
#[cfg(feature = "engines")]
pub mod ci;
#[cfg(feature = "engines")]
pub mod cloudflare;
#[cfg(feature = "engines")]
pub mod config;
#[cfg(feature = "engines")]
mod engines;
#[cfg(feature = "engines")]
pub mod generations;
#[cfg(feature = "engines")]
pub mod git;
#[cfg(any(feature = "engines", feature = "worker"))]
pub mod link;
#[cfg(feature = "engines")]
pub mod manifest;
#[cfg(feature = "engines")]
pub mod moves;
#[cfg(feature = "engines")]
pub mod plans;
#[cfg(feature = "engines")]
pub mod providers;
#[cfg(feature = "engines")]
pub mod release;
#[cfg(feature = "engines")]
pub mod s3;
#[cfg(feature = "engines")]
pub mod scope;
#[cfg(feature = "engines")]
pub mod secrets;
#[cfg(feature = "engines")]
pub mod session;
#[cfg(feature = "engines")]
pub mod ship;
#[cfg(any(feature = "engines", feature = "worker"))]
pub mod ship_wire;
#[cfg(feature = "engines")]
pub mod sites;
#[cfg(feature = "engines")]
pub mod state;
#[cfg(feature = "engines")]
pub mod static_site;
#[cfg(feature = "engines")]
mod tofu;
#[cfg(feature = "engines")]
pub mod unit;
#[cfg(feature = "worker")]
pub mod worker;
#[cfg(feature = "engines")]
pub mod workers;

#[cfg(feature = "engines")]
pub use engines::*;

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
