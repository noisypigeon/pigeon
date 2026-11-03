//! `pigeon backplane` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon backplane` commands.
#[derive(Subcommand, Debug)]
pub enum BackplaneCommand {
    /// Validate and show the instance manifest; exit 2 if invalid
    Instance(RootArgs),
    /// List the engines, their versions and roles
    Engines(JsonArgs),
    /// List the operations a surface may request
    Operations(JsonArgs),
    /// Run an operation; exit 1 on a violation, 2 on an error
    Run(RunArgs),
}

/// `--json` alone.
#[derive(Args, Debug)]
pub struct JsonArgs {
    /// Print JSON
    #[arg(long)]
    pub json: bool,
}

/// An instance root and `--json`.
#[derive(Args, Debug)]
pub struct RootArgs {
    /// The instance's root, holding `.config/instance.yaml`
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// Print the JSON response
    #[arg(long)]
    pub json: bool,
}

/// `pigeon backplane run` arguments.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// The operation, as `pigeon backplane operations` lists it
    pub operation: String,

    /// The instance root and output format
    #[command(flatten)]
    pub root: RootArgs,
}
