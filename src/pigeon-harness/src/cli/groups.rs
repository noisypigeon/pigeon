//! The command groups that take their arguments from the old `cobot` and `backplane` ones.

#[cfg(feature = "agent")]
use clap::Args;
#[cfg(any(feature = "infra", feature = "agent"))]
use clap::Subcommand;

#[cfg(feature = "infra")]
use super::{
    InfraApply, InfraPlan, InfraRoot, InfraRun, InfraShow, ReleaseArgs, SecretsCommand, TagsArgs,
};
#[cfg(feature = "agent")]
use super::{JobsArgs, ServeArgs};

/// `pigeon infra` commands.
#[cfg(feature = "infra")]
#[derive(Subcommand, Debug)]
pub enum InfraCommand {
    /// Check every unit under infra/ and workloads/; exit 1 on a problem
    Validate(InfraRoot),
    /// Write each unit's main.tf.json under <ROOT>/.pigeon/
    Render(InfraRun),
    /// Plan units against the state repository; print a summary
    Plan(InfraPlan),
    /// Plan, then apply units with changes
    Apply(InfraApply),
    /// Print a stored plan's changes attribute by attribute; sensitive values stay hidden
    Show(InfraShow),
    /// Write <PROVIDERS>/providers.lock.hcl for the allowlist
    Lock(InfraRoot),
    /// Print each catalog's capabilities as JSON; exit 1 on an invalid one
    Capabilities(InfraRoot),
    /// Manage a unit's encrypted secrets.config.yaml
    #[command(subcommand)]
    Secrets(SecretsCommand),
}

/// `pigeon release`: cut releases, or `pigeon release tags`.
#[cfg(feature = "infra")]
#[derive(clap::Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ReleaseGroup {
    /// `tags`, instead of cutting releases
    #[command(subcommand)]
    pub command: Option<ReleaseCommand>,
    /// The release's arguments
    #[command(flatten)]
    pub args: ReleaseArgs,
}

/// `pigeon release` subcommands.
#[cfg(feature = "infra")]
#[derive(Subcommand, Debug)]
pub enum ReleaseCommand {
    /// Print every release tag as JSON, flagging each unit's newest
    Tags(TagsArgs),
}

/// `pigeon agent` commands.
#[cfg(feature = "agent")]
#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Answer operations over HTTP (GET /healthz, GET /v0/operations, POST /v0/run); needs
    /// `AGENT_TOKEN`
    Serve(ServeArgs),
    /// Take a change's jobs over signed HTTP (POST /v0/jobs, GET /v0/jobs/<id>), in a job
    /// Container; needs `AGENT_TOKEN`
    Listen(ListenArgs),
}

/// `pigeon agent listen` arguments: `pigeon agent listen`'s, with the address also positional.
#[cfg(feature = "agent")]
#[derive(Args, Debug)]
pub struct ListenArgs {
    /// The address to listen on, e.g. 0.0.0.0:8080
    #[arg(required_unless_present = "listen", conflicts_with = "listen")]
    pub address: Option<String>,
    /// The jobs' arguments
    #[command(flatten)]
    pub jobs: JobsArgsOptional,
}

/// [`JobsArgs`] with `--listen` optional, since [`ListenArgs`] also takes it positionally.
#[cfg(feature = "agent")]
#[derive(Args, Debug)]
pub struct JobsArgsOptional {
    /// The address to listen on, e.g. 0.0.0.0:8080
    #[arg(long)]
    pub listen: Option<String>,
    /// Where jobs check out [default: the temporary directory]
    #[arg(long)]
    pub work: Option<std::path::PathBuf>,
    /// Also serve `/v0/bench/*`: a session's bench, whose commands run as uid 1000 with no
    /// network; see .claude/decisions/2026-11-16-development-session-bench-containers.md
    #[arg(long)]
    pub bench: bool,
}

#[cfg(feature = "agent")]
impl ListenArgs {
    /// The `pigeon agent listen` arguments these are.
    #[must_use]
    pub fn into_jobs(self) -> JobsArgs {
        JobsArgs {
            listen: self.address.or(self.jobs.listen).unwrap_or_default(),
            work: self.jobs.work,
            bench: self.jobs.bench,
        }
    }
}

/// `pigeon keys` commands.
#[cfg(feature = "agent")]
#[derive(Subcommand, Debug)]
pub enum KeysCommand {
    /// Print a new authenticator secret for approvals in the card: base32 on stdout, for
    /// `pigeon workers secrets put … APPROVER_TOTP`, and its `otpauth://` URI on stderr, for
    /// the authenticator app
    Totp,
    /// Print a one-time passkey enrolment: its stored form on stdout, for
    /// `pigeon workers secrets put … PASSKEY_ENROLL`, and its token on stderr, to type at
    /// `/passkeys/enroll` within the hour
    Enroll,
    /// Print a new Web Push key: its private half, base64url, on stdout, for
    /// `pigeon workers secrets put … PUSH_VAPID_KEY`, and its public half on stderr
    Vapid,
}
