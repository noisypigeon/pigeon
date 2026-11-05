//! `pigeon cobot` arguments.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// `pigeon cobot` commands.
#[derive(Subcommand, Debug)]
pub enum CobotCommand {
    /// Check every unit under infra/ and workloads/; exit 1 on a problem
    Validate(CobotRoot),
    /// Write each unit's main.tf.json under <ROOT>/.cobot/
    Render(CobotRun),
    /// Plan units against the state repository; print a summary
    Plan(CobotPlan),
    /// Plan, then apply units with changes
    Apply(CobotApply),
    /// Write <PROVIDERS>/providers.lock.hcl for the allowlist
    Lock(CobotRoot),
    /// Print each catalog's capabilities as JSON; exit 1 on an invalid one
    Capabilities(CobotRoot),
    /// Manage a unit's encrypted secrets.config.yaml
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// Build static sites, or publish them to their R2 buckets
    #[command(subcommand)]
    Sites(SitesCommand),
    /// Bump changed release units and generate changelogs; print JSON
    Release(ReleaseArgs),
    /// Print every release tag as JSON, flagging each unit's newest
    Tags(TagsArgs),
}

/// The repository root and its provider allowlist.
#[derive(Args, Debug)]
pub struct CobotRoot {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The directory holding providers.yaml and providers.lock.hcl [default: ./.config]
    #[arg(long)]
    pub providers: Option<PathBuf>,
}

impl CobotRoot {
    /// The providers directory.
    #[must_use]
    pub fn providers_dir(&self) -> PathBuf {
        self.providers
            .clone()
            .unwrap_or_else(|| PathBuf::from(".config"))
    }
}

/// Units and their state.
#[derive(Args, Debug)]
pub struct CobotRun {
    /// The repository root and providers
    #[command(flatten)]
    pub at: CobotRoot,
    /// The state repository's checkout, or `r2://<bucket>/<prefix>` with `COBOT_STATE_R2_*` set
    #[arg(long)]
    pub state: PathBuf,
    /// Unit paths, e.g. infra/bootstrap [default: every unit]
    pub units: Vec<String>,
}

/// `pigeon cobot plan` arguments.
#[derive(Args, Debug)]
pub struct CobotPlan {
    /// Units and their state
    #[command(flatten)]
    pub run: CobotRun,
    /// Also write the Markdown summary here
    #[arg(long)]
    pub summary: Option<PathBuf>,
    /// Store the plan in the state for `apply --plan`, valid for 24 hours
    #[arg(long)]
    pub store: bool,
}

/// `pigeon cobot apply` arguments.
#[derive(Args, Debug)]
pub struct CobotApply {
    /// Units and their state
    #[command(flatten)]
    pub run: CobotRun,
    /// Apply exactly this stored plan, from `plan --store`, instead of planning again
    #[arg(long, conflicts_with = "units")]
    pub plan: Option<String>,
}

/// `pigeon cobot secrets` commands; there is deliberately no `get`.
#[derive(Subcommand, Debug)]
pub enum SecretsCommand {
    /// Encrypt a value read from stdin into <UNIT>/secrets.config.yaml
    Set(SecretsKey),
    /// List the keys and verify each decrypts; never prints a value
    Check(SecretsUnit),
}

/// `pigeon cobot sites` commands.
#[derive(Subcommand, Debug)]
pub enum SitesCommand {
    /// Build each static site with its pinned template; needs no secrets
    Build(SitesBuild),
    /// Build, sync each site to its bucket, then check it live
    Publish(SitesPublish),
}

/// `pigeon cobot sites build` arguments.
#[derive(Args, Debug)]
pub struct SitesBuild {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Workload paths, e.g. workloads/noisypigeon.com/blog [default: every static site]
    pub workloads: Vec<String>,
    /// A JSON file each site's template reads as `data/<NAME>.json`, as NAME=FILE; repeatable
    #[arg(long = "data", value_name = "NAME=FILE")]
    pub data: Vec<String>,
}

/// `pigeon cobot sites publish` arguments.
#[derive(Args, Debug)]
pub struct SitesPublish {
    /// Workloads and their state
    #[command(flatten)]
    pub run: CobotRun,
    /// How many times each live check is tried, 15 seconds apart; 0 skips them
    #[arg(long, default_value_t = 12)]
    pub checks: u32,
    /// A JSON file each site's template reads as `data/<NAME>.json`, as NAME=FILE; repeatable
    #[arg(long = "data", value_name = "NAME=FILE")]
    pub data: Vec<String>,
}

/// A unit's secrets file.
#[derive(Args, Debug)]
pub struct SecretsUnit {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The unit path, e.g. infra/cloudflare
    pub unit: String,
}

/// One key in a unit's secrets file.
#[derive(Args, Debug)]
pub struct SecretsKey {
    /// The unit
    #[command(flatten)]
    pub at: SecretsUnit,
    /// The key
    pub key: String,
}

/// `pigeon cobot tags` arguments.
#[derive(Args, Debug)]
pub struct TagsArgs {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,

    /// The release configuration [default: <ROOT>/.config/releases.yaml]
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// `pigeon cobot release` arguments.
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
