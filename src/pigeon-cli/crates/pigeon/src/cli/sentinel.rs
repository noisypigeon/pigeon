//! `pigeon sentinel` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon sentinel` commands.
#[derive(Subcommand, Debug)]
pub enum SentinelCommand {
    /// Run every policy; exit 1 on a violation, 2 on an invalid policy
    Check(CheckArgs),
    /// Bump changed release units and generate changelogs; print JSON
    Release(ReleaseArgs),
    /// Print every release tag as JSON, flagging each unit's newest
    Tags(TagsArgs),
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

/// `pigeon sentinel tags` arguments.
#[derive(Args, Debug)]
pub struct TagsArgs {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// The release configuration [default: <ROOT>/.config/releases.yaml]
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// `pigeon sentinel release` arguments.
#[derive(Args, Debug)]
pub struct ReleaseArgs {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// The release configuration [default: <ROOT>/.config/releases.yaml]
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Start after this commit [default: the last `chore(release):` commit, or all history]
    #[arg(long)]
    pub from: Option<String>,

    /// The commit to release up to
    #[arg(long, default_value = "HEAD")]
    pub to: String,

    /// Write each release's notes to `<DIR>/<unit>.md`
    #[arg(long)]
    pub notes_dir: Option<PathBuf>,

    /// Print what would happen; write nothing
    #[arg(long)]
    pub dry_run: bool,
}
