//! Command-line arguments; each command group is a cargo feature, and `dev` is always built.
//! The groups are .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §3's;
//! the old spellings stay as hidden aliases until its step 12.

use clap::{Parser, Subcommand};

#[cfg(any(feature = "agent", feature = "signing"))]
mod backplane;
#[cfg(feature = "infra")]
mod cobot;
mod dev;
mod groups;
#[cfg(feature = "eviction")]
mod sentinel;
#[cfg(feature = "session")]
mod session;

#[cfg(any(feature = "agent", feature = "signing"))]
pub use backplane::*;
#[cfg(feature = "infra")]
pub use cobot::*;
pub use dev::*;
#[cfg(any(feature = "infra", feature = "agent"))]
pub use groups::*;
#[cfg(feature = "eviction")]
pub use sentinel::*;
#[cfg(feature = "session")]
pub use session::*;

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
    /// Check the repository against its policies
    #[cfg(feature = "eviction")]
    #[command(subcommand)]
    Eviction(SentinelCommand),
    /// Validate, render, plan and apply infrastructure units, and manage their secrets
    #[cfg(feature = "infra")]
    #[command(subcommand, version = ::backplane::cobot::VERSION)]
    Infra(InfraCommand),
    /// Build static sites, or publish them to their R2 buckets
    #[cfg(feature = "infra")]
    #[command(subcommand)]
    Sites(SitesCommand),
    /// Host the instance's git repositories on Cloudflare Artifacts, mirrored to GitHub
    #[cfg(feature = "infra")]
    #[command(subcommand)]
    Git(GitCommand),
    /// Render, plan and deploy the platform's Workers with wrangler
    #[cfg(feature = "infra")]
    #[command(subcommand)]
    Workers(WorkersCommand),
    /// Bump changed release units and generate changelogs; print JSON
    #[cfg(feature = "infra")]
    Release(ReleaseGroup),
    /// Validate and show the instance manifest; exit 2 if invalid
    #[cfg(feature = "agent")]
    Instance(RootArgs),
    /// List the engines, their versions and roles
    #[cfg(feature = "agent")]
    Engines(JsonArgs),
    /// List the operations a surface may request
    #[cfg(feature = "agent")]
    Ops(JsonArgs),
    /// Run an operation; exit 1 on a violation, 2 on an error
    #[cfg(feature = "agent")]
    Run(RunArgs),
    /// List the ADRs that govern each path, by their `Governs` globs
    #[cfg(feature = "agent")]
    Decisions(DecisionsArgs),
    /// Serve operations and a change's jobs over HTTP
    #[cfg(feature = "agent")]
    #[command(subcommand, version = ::backplane::VERSION)]
    Agent(AgentCommand),
    /// Print new secrets for the control plane's Worker
    #[cfg(feature = "agent")]
    #[command(subcommand)]
    Keys(KeysCommand),
    /// Load the release's signing identity into a throwaway keychain, upload the signed app,
    /// and clean up, in Buildkite's `sign` step
    #[cfg(feature = "signing")]
    #[command(subcommand)]
    Signing(SigningCommand),
    /// Follow this agent session's events, brief it, and feed its status line
    #[cfg(feature = "session")]
    #[command(subcommand)]
    Session(SessionCommand),
    /// Build, test and check the workspace: its tasks, run from the workspace root
    Dev(DevArgs),
    /// `pigeon eviction`'s old spelling
    #[cfg(feature = "eviction")]
    #[command(subcommand, hide = true)]
    Sentinel(SentinelCommand),
    /// The old spellings of `pigeon infra`, `sites`, `git`, `workers`, `release` and `dev scope`
    #[cfg(feature = "infra")]
    #[command(subcommand, hide = true, version = ::backplane::cobot::VERSION)]
    Cobot(CobotCommand),
    /// The old spellings of `pigeon instance`, `engines`, `ops`, `run`, `decisions`, `agent`,
    /// `keys` and `signing`
    #[cfg(any(feature = "agent", feature = "signing"))]
    #[command(subcommand, hide = true, version = ::backplane::VERSION)]
    Backplane(BackplaneCommand),
}
