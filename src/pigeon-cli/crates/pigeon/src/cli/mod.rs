//! Command-line arguments; each subcommand is a cargo feature.

use clap::{Parser, Subcommand};

#[cfg(feature = "cobot")]
mod cobot;
#[cfg(feature = "sentinel")]
mod sentinel;

#[cfg(feature = "cobot")]
pub use cobot::*;
#[cfg(feature = "sentinel")]
pub use sentinel::*;

/// The pigeon command-line tool.
#[derive(Parser, Debug)]
#[command(name = "pigeon", version, about, long_about = None)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level commands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Check the repository against its policies
    #[cfg(feature = "sentinel")]
    #[command(subcommand, version = ::sentinel::VERSION)]
    Sentinel(SentinelCommand),
    /// Not implemented yet
    #[cfg(feature = "gantry")]
    #[command(version = gantry::VERSION)]
    Gantry,
    /// Validate, render, plan and apply infrastructure units, and cut releases
    #[cfg(feature = "cobot")]
    #[command(subcommand, version = ::cobot::VERSION)]
    Cobot(CobotCommand),
}
