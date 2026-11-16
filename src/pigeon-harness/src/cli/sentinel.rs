//! `pigeon sentinel` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon sentinel` commands.
#[derive(Subcommand, Debug)]
pub enum SentinelCommand {
    /// Run every policy; exit 1 on a violation, 2 on an invalid policy
    Check(CheckArgs),
}

/// `pigeon sentinel check` arguments.
#[derive(Args, Debug)]
pub struct CheckArgs {
    /// The repository root that policies resolve paths against
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// The policy directory [default: <ROOT>/.config/policies]
    #[arg(long)]
    pub policies: Option<PathBuf>,

    /// Also run pull-request policies, reading `PR_TITLE`, `PR_BODY`, `PR_NUMBER` and `PR_FILES`
    #[arg(long)]
    pub pull_request: bool,
}
