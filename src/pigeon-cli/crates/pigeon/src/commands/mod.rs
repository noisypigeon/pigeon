//! Command dispatch.

#[cfg(feature = "backplane")]
pub mod backplane;
#[cfg(feature = "cobot")]
pub mod cobot;
#[cfg(feature = "sentinel")]
pub mod sentinel;
#[cfg(feature = "signing")]
pub mod signing;
#[cfg(feature = "cobot")]
pub mod workers;

use crate::cli::Commands;

/// Every policy held, or the command succeeded.
pub const SUCCESS: i32 = 0;
/// A policy was violated.
pub const VIOLATION: i32 = 1;
/// An invalid policy or a usage error.
pub const INVALID: i32 = 2;

/// Runs a command and returns its exit code.
#[must_use]
#[cfg_attr(
    not(any(
        feature = "sentinel",
        feature = "cobot",
        feature = "backplane",
        feature = "signing"
    )),
    expect(
        clippy::needless_pass_by_value,
        reason = "only `sentinel`, `cobot`, `backplane` and `signing` carry arguments"
    )
)]
pub fn dispatch(command: Commands) -> i32 {
    match command {
        #[cfg(feature = "sentinel")]
        Commands::Sentinel(c) => sentinel::run(c),
        #[cfg(feature = "cobot")]
        Commands::Cobot(c) => cobot::run(c),
        #[cfg(feature = "backplane")]
        Commands::Backplane(c) => backplane::run(c),
        #[cfg(all(feature = "signing", not(feature = "backplane")))]
        Commands::Backplane(crate::cli::BackplaneCommand::Signing(c)) => signing::run(c),
    }
}
