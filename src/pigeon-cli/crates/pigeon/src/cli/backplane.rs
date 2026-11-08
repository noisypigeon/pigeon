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
    /// Take a change's jobs over signed HTTP (POST /v0/jobs, GET /v0/jobs/<id>), in a job
    /// Container; needs `BACKPLANE_TOKEN`
    Jobs(JobsArgs),
    /// Print a new authenticator secret for approvals in the card: base32 on stdout, for
    /// `pigeon cobot workers secrets put … APPROVER_TOTP`, and its `otpauth://` URI on stderr,
    /// for the authenticator app
    TotpSecret,
    /// Print a new Web Push key: its private half, base64url, on stdout, for
    /// `pigeon cobot workers secrets put … PUSH_VAPID_KEY`, and its public half on stderr
    VapidKey,
}

/// `pigeon backplane jobs` arguments.
#[derive(Args, Debug)]
pub struct JobsArgs {
    /// The address to listen on, e.g. 0.0.0.0:8080
    #[arg(long)]
    pub listen: String,
    /// Where jobs check out [default: the temporary directory]
    #[arg(long)]
    pub work: Option<PathBuf>,
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
    /// `BACKPLANE_TLS_KEY` and `BACKPLANE_TLS_CLIENT_CA` (PEM), or --signed
    #[arg(long)]
    pub network: bool,

    /// Take signed requests on `/v0/` routes instead of the bearer token, and sign each
    /// response; the token is the signing key
    #[arg(long)]
    pub signed: bool,

    /// The platform's root; also the instance's, unless --instance names another
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// Another instance's root, holding its `.config/instance.yaml`
    #[arg(long)]
    pub instance: Option<PathBuf>,
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
    /// The platform's root; also the instance's, unless --instance names another
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// Another instance's root, holding its `.config/instance.yaml`; catalogs, providers and
    /// templates still come from --root
    #[arg(long)]
    pub instance: Option<PathBuf>,

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

    /// An argument as `name=value`, always a string, never parsed as JSON
    #[arg(long = "text", value_name = "NAME=VALUE")]
    pub text: Vec<String>,

    /// The instance root and output format
    #[command(flatten)]
    pub root: RootArgs,
}
