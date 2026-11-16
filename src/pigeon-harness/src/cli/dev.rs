//! `pigeon dev` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon dev <task>`, or `pigeon dev scope`.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true, subcommand_required = false)]
pub struct DevArgs {
    /// `scope`, which takes its own arguments
    #[command(subcommand)]
    pub command: Option<DevCommand>,
    /// The task, as `pigeon dev --list` names it; its dependencies run first
    #[arg(required_unless_present = "list", value_parser = clap::builder::PossibleValuesParser::new(crate::dev::names()))]
    pub task: Option<String>,
    /// Print the commands, in order, instead of running them
    #[arg(long)]
    pub dry_run: bool,
    /// List the tasks and what each depends on
    #[arg(long, conflicts_with = "task")]
    pub list: bool,
    /// The repository root [default: the nearest directory up holding a workspace `Cargo.toml`]
    #[arg(long)]
    pub root: Option<PathBuf>,
}

/// `pigeon dev` subcommands.
#[derive(Subcommand, Debug)]
pub enum DevCommand {
    /// Print the checks a change needs, from the paths it touches; with --run, run them
    #[cfg(feature = "infra")]
    Scope(super::ScopeArgs),
}
