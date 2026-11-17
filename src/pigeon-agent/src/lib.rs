//! Composes the engines into one platform; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.
//! Core's pure modules are re-exported at their old paths; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §2.

pub use pigeon_core as core;
pub use pigeon_core::{
    access, actions, apple, approve, briefs, ci_window, control, decisions, events, gitcreds,
    gitdoor, iosrelease, mcp, mirroring, push, reconcile, routes, sessions, signing, tasks,
    tickets, wire,
};

pub mod architecture;
#[cfg(feature = "engines")]
pub mod bench;
#[cfg(feature = "engines")]
pub mod catalog;
#[cfg(feature = "engines")]
pub mod checks;
#[cfg(any(feature = "worker", test))]
pub mod containers;
#[cfg(feature = "engines")]
pub mod engine;
#[cfg(feature = "engines")]
pub mod eviction;
#[cfg(feature = "engines")]
pub mod git;
#[cfg(feature = "engines")]
pub mod infra;
#[cfg(feature = "engines")]
pub mod instance;
#[cfg(feature = "engines")]
pub mod jobs;
pub mod names;
#[cfg(feature = "engines")]
pub mod platform_job;
#[cfg(feature = "engines")]
pub mod release;
#[cfg(feature = "engines")]
pub mod serve;
#[cfg(feature = "engines")]
pub mod ship;
#[cfg(feature = "engines")]
pub mod sites;
#[cfg(feature = "engines")]
pub mod standup_job;
#[cfg(feature = "worker")]
pub mod worker;
#[cfg(feature = "engines")]
pub mod workers;

#[cfg(feature = "engines")]
pub use catalog::{run, run_with};
#[cfg(feature = "engines")]
pub use engine::{EngineInfo, engines};
#[cfg(feature = "engines")]
pub use instance::Instance;
pub use wire::{Effect, Engine, Error, Operation, Request, Response, catalog, validate};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
