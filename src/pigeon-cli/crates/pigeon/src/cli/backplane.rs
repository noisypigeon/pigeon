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
    /// Answer operations over HTTP (GET /healthz, GET /v0/operations, POST /v0/run); needs
    /// `BACKPLANE_TOKEN`
    Serve(ServeArgs),
}

/// `pigeon backplane serve` arguments.
#[derive(Args, Debug)]
pub struct ServeArgs {
    /// The address to listen on, e.g. 127.0.0.1:8787; there is no default
    #[arg(long)]
    pub listen: String,

    /// The state every plan and apply uses: a directory, or `r2://<bucket>/<prefix>`
    #[arg(long)]
    pub state: String,

    /// Allow a non-loopback address; requires mutual TLS from `BACKPLANE_TLS_CERT`,
    /// `BACKPLANE_TLS_KEY` and `BACKPLANE_TLS_CLIENT_CA` (PEM)
    #[arg(long)]
    pub network: bool,

    /// The instance's root, holding `.config/instance.yaml`
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
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

    /// An argument as `name=value`; a value that parses as JSON is JSON, else a string
    #[arg(long = "arg", value_name = "NAME=VALUE")]
    pub args: Vec<String>,

    /// The instance root and output format
    #[command(flatten)]
    pub root: RootArgs,
}
