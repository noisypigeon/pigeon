//! Command-line arguments; each subcommand is a cargo feature.

#[cfg(any(feature = "sentinel", feature = "cobot"))]
use std::path::PathBuf;

#[cfg(any(feature = "sentinel", feature = "cobot"))]
use clap::Args;
use clap::{Parser, Subcommand};

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
    /// Check the repository against its policies, and cut releases
    #[cfg(feature = "sentinel")]
    #[command(subcommand, version = sentinel::VERSION)]
    Sentinel(SentinelCommand),
    /// Not implemented yet
    #[cfg(feature = "gantry")]
    #[command(version = gantry::VERSION)]
    Gantry,
    /// Validate, render, plan and apply infrastructure units
    #[cfg(feature = "cobot")]
    #[command(subcommand, version = cobot::VERSION)]
    Cobot(CobotCommand),
}

/// `pigeon cobot` commands.
#[cfg(feature = "cobot")]
#[derive(Subcommand, Debug)]
pub enum CobotCommand {
    /// Check every unit under infra/ and workloads/; exit 1 on a problem
    Validate(CobotRoot),
    /// Write each unit's main.tf.json under <ROOT>/.cobot/
    Render(CobotRun),
    /// Plan units against the state repository; print a summary
    Plan(CobotPlan),
    /// Plan, then apply units with changes
    Apply(CobotRun),
}

/// The repository root.
#[cfg(feature = "cobot")]
#[derive(Args, Debug)]
pub struct CobotRoot {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
}

/// Units and their state.
#[cfg(feature = "cobot")]
#[derive(Args, Debug)]
pub struct CobotRun {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The state repository's checkout
    #[arg(long)]
    pub state: PathBuf,
    /// Unit paths, e.g. infra/bootstrap [default: every unit]
    pub units: Vec<String>,
}

/// `pigeon cobot plan` arguments.
#[cfg(feature = "cobot")]
#[derive(Args, Debug)]
pub struct CobotPlan {
    /// Units and their state
    #[command(flatten)]
    pub run: CobotRun,
    /// Also write the Markdown summary here
    #[arg(long)]
    pub summary: Option<PathBuf>,
}

/// `pigeon sentinel` commands.
#[cfg(feature = "sentinel")]
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
#[cfg(feature = "sentinel")]
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
#[cfg(feature = "sentinel")]
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
#[cfg(feature = "sentinel")]
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
