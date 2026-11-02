//! The `pigeon` binary.

use clap::Parser;
use pigeon::cli::Cli;

fn main() {
    std::process::exit(pigeon::commands::dispatch(Cli::parse().command));
}
