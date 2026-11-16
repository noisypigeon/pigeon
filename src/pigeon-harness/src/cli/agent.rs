//! `pigeon agent` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon agent` commands.
#[derive(Subcommand, Debug)]
pub enum AgentOp {
    /// Validate and show the instance manifest; exit 2 if invalid
    #[cfg(feature = "agent")]
    Instance(RootArgs),
    /// List the engines, their versions and roles
    #[cfg(feature = "agent")]
    Engines(JsonArgs),
    /// List the operations a surface may request
    #[cfg(feature = "agent")]
    Operations(JsonArgs),
    /// Run an operation; exit 1 on a violation, 2 on an error
    #[cfg(feature = "agent")]
    Run(RunArgs),
    /// List the ADRs that govern each path, by their `Governs` globs
    #[cfg(feature = "agent")]
    Decisions(DecisionsArgs),
    /// Answer operations over HTTP (GET /healthz, GET /v0/operations, POST /v0/run); needs
    /// `BACKPLANE_TOKEN`
    #[cfg(feature = "agent")]
    Serve(ServeArgs),
    /// Take a change's jobs over signed HTTP (POST /v0/jobs, GET /v0/jobs/<id>), in a job
    /// Container; needs `BACKPLANE_TOKEN`
    #[cfg(feature = "agent")]
    Jobs(JobsArgs),
    /// Print a new authenticator secret for approvals in the card: base32 on stdout, for
    /// `pigeon workers secrets put … APPROVER_TOTP`, and its `otpauth://` URI on stderr,
    /// for the authenticator app
    #[cfg(feature = "agent")]
    TotpSecret,
    /// Print a one-time passkey enrolment: its stored form on stdout, for
    /// `pigeon workers secrets put … PASSKEY_ENROLL`, and its token on stderr, to type at
    /// `/passkeys/enroll` within the hour
    #[cfg(feature = "agent")]
    EnrollSecret,
    /// Print a new Web Push key: its private half, base64url, on stdout, for
    /// `pigeon workers secrets put … PUSH_VAPID_KEY`, and its public half on stderr
    #[cfg(feature = "agent")]
    VapidKey,
    /// Load the release's signing identity into a throwaway keychain, upload the signed app,
    /// and clean up, in Buildkite's `sign` step
    #[cfg(feature = "signing")]
    #[command(subcommand)]
    Signing(SigningCommand),
}

/// `pigeon decisions` arguments.
#[derive(Args, Debug)]
pub struct DecisionsArgs {
    /// Repository paths
    #[arg(required = true)]
    pub paths: Vec<String>,
    /// The repository's root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Print JSON
    #[arg(long)]
    pub json: bool,
}

/// `pigeon agent listen` arguments.
#[derive(Args, Debug)]
pub struct JobsArgs {
    /// The address to listen on, e.g. 0.0.0.0:8080
    #[arg(long)]
    pub listen: String,
    /// Where jobs check out [default: the temporary directory]
    #[arg(long)]
    pub work: Option<PathBuf>,
    /// Also serve `/v0/bench/*`: a session's bench, whose commands run as uid 1000 with no
    /// network; see .claude/decisions/2026-11-16-development-session-bench-containers.md
    #[arg(long)]
    pub bench: bool,
}

/// `pigeon agent serve` arguments.
#[derive(Args, Debug)]
pub struct ServeArgs {
    /// The address to listen on, e.g. 127.0.0.1:8787; there is no default
    #[arg(long)]
    pub listen: String,

    /// The state every plan and apply uses: a directory, or `r2://<bucket>/<prefix>`
    #[arg(long)]
    pub state: String,

    /// Allow a non-loopback address; requires mutual TLS from `AGENT_TLS_CERT`,
    /// `AGENT_TLS_KEY` and `AGENT_TLS_CLIENT_CA` (PEM), or --signed
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

/// `pigeon run` arguments.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// The operation, as `pigeon ops` lists it
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

/// `pigeon signing` commands.
#[cfg(feature = "signing")]
#[derive(Subcommand, Debug)]
pub enum SigningCommand {
    /// Exchange the job's OIDC token for the signing identity and load it; prints the
    /// `ExportOptions.plist` path
    Load(SigningArgs),
    /// Upload a signed `.ipa` to App Store Connect with the grant's token
    Upload(UploadArgs),
    /// Undo `load`; succeeds when there is nothing to undo
    Clean(SigningArgs),
}

/// Where the harness keeps its files.
#[cfg(feature = "signing")]
#[derive(Args, Debug)]
pub struct SigningArgs {
    /// The control plane to ask
    #[arg(long, default_value = backplane::signing::PLATFORM)]
    pub platform: String,
}

/// `pigeon signing upload` arguments.
#[cfg(feature = "signing")]
#[derive(Args, Debug)]
pub struct UploadArgs {
    /// The signed app
    #[arg(long)]
    pub ipa: PathBuf,
    /// Its marketing version, `CFBundleShortVersionString`
    #[arg(long)]
    pub version: String,
    /// Its build number, `CFBundleVersion`
    #[arg(long)]
    pub build: u64,
}
