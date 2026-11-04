//! Composes the engines into one platform; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

pub mod catalog;
pub mod engine;
pub mod instance;
pub mod serve;

use std::fmt;
use std::path::PathBuf;

pub use catalog::{Effect, Operation, Request, Response, catalog, run, run_with, validate};
pub use engine::{Engine, EngineInfo, engines};
pub use instance::Instance;

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
