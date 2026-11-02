//! Command dispatch.

pub mod sentinel;

use crate::cli::Commands;

/// Every policy held, or the command succeeded.
pub const SUCCESS: i32 = 0;
/// A policy was violated.
pub const VIOLATION: i32 = 1;
/// An invalid policy or a usage error.
pub const INVALID: i32 = 2;

/// Runs a command and returns its exit code.
#[must_use]
pub fn dispatch(command: Commands) -> i32 {
    match command {
        Commands::Sentinel(c) => sentinel::run(c),
    }
}
