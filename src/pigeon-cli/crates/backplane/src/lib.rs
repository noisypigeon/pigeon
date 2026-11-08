//! Composes the engines into one platform; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

pub mod access;
pub mod approvals;
pub mod approve;
pub mod architecture;
#[cfg(feature = "engines")]
pub mod catalog;
pub mod changes;
pub mod control;
#[cfg(feature = "engines")]
pub mod engine;
pub mod events;
pub mod gitdoor;
pub mod grants;
#[cfg(feature = "engines")]
pub mod instance;
#[cfg(feature = "engines")]
pub mod jobs;
pub mod mandates;
pub mod mcp;
pub mod merges;
pub mod push;
pub mod reconcile;
pub mod runs;
#[cfg(feature = "engines")]
pub mod serve;
pub mod sessions;
#[cfg(feature = "engines")]
pub mod ship;
pub mod tasks;
pub mod tickets;
pub mod totp;
pub mod versions;
pub mod wire;
#[cfg(feature = "worker")]
pub mod worker;

use std::fmt;
use std::path::PathBuf;

#[cfg(feature = "engines")]
pub use catalog::{run, run_with};
#[cfg(feature = "engines")]
pub use engine::{EngineInfo, engines};
#[cfg(feature = "engines")]
pub use instance::Instance;
pub use wire::{Effect, Engine, Operation, Request, Response, catalog, validate};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why an operation could not run.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read.
    Io {
        /// The path.
        path: PathBuf,
        /// Why.
        message: String,
    },
    /// The manifest or a request is invalid; every problem found.
    Invalid(Vec<String>),
    /// An engine failed.
    Engine {
        /// The engine.
        engine: Engine,
        /// Its error.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Invalid(problems) => {
                write!(f, "{} problem(s):", problems.len())?;
                for p in problems {
                    write!(f, "\n{p}")?;
                }
                Ok(())
            }
            Self::Engine { engine, message } => write!(f, "{}: {message}", engine.name()),
        }
    }
}

impl std::error::Error for Error {}
