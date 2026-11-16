//! Composes the engines into one platform; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

pub mod access;
pub mod actions;
pub mod apple;
pub mod approvals;
pub mod approve;
pub mod architecture;
pub mod briefs;
#[cfg(feature = "engines")]
pub mod catalog;
pub mod changes;
pub mod ci_window;
pub mod cobot;
pub mod control;
pub mod core;
pub mod decisions;
pub mod devices;
#[cfg(feature = "engines")]
pub mod engine;
pub mod events;
pub mod gitcreds;
pub mod gitdoor;
pub mod grants;
pub mod insights;
#[cfg(feature = "engines")]
pub mod instance;
pub mod iosrelease;
#[cfg(feature = "engines")]
pub mod jobs;
pub mod macos;
pub mod mandates;
pub mod mcp;
pub mod merges;
pub mod mirroring;
pub mod names;
pub mod passkeys;
#[cfg(feature = "engines")]
pub mod platform_job;
pub mod platforms;
pub mod push;
pub mod reconcile;
pub mod routes;
pub mod runs;
#[cfg(feature = "engines")]
pub mod sentinel;
#[cfg(feature = "engines")]
pub mod serve;
pub mod sessions;
#[cfg(feature = "engines")]
pub mod ship;
pub mod signing;
#[cfg(feature = "engines")]
pub mod standup_job;
pub mod tasks;
pub mod testmaps;
pub mod tickets;
pub mod totp;
pub mod versions;
pub mod wire;
#[cfg(feature = "worker")]
pub mod worker;

#[cfg(feature = "engines")]
pub use catalog::{run, run_with};
#[cfg(feature = "engines")]
pub use engine::{EngineInfo, engines};
#[cfg(feature = "engines")]
pub use instance::Instance;
pub use wire::{Effect, Engine, Error, Operation, Request, Response, catalog, validate};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
