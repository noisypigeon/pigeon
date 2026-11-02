//! The `pigeon` binary.

use clap::Parser;
use pigeon::cli::Cli;

#[cfg_attr(
    not(any(feature = "sentinel", feature = "gantry")),
    expect(unreachable_code, reason = "no subcommands are built")
)]
fn main() {
    std::process::exit(pigeon::commands::dispatch(Cli::parse().command));
}
