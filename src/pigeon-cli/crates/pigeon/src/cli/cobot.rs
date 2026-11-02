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
    Apply(CobotRun),
    /// Write <PROVIDERS>/providers.lock.hcl for the allowlist
    Lock(CobotRoot),
    /// Manage a unit's encrypted secrets.config.yaml
    #[command(subcommand)]
    Secrets(SecretsCommand),
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
    /// The state repository's checkout
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
}

/// `pigeon cobot secrets` commands; there is deliberately no `get`.
#[derive(Subcommand, Debug)]
pub enum SecretsCommand {
    /// Encrypt a value read from stdin into <UNIT>/secrets.config.yaml
    Set(SecretsKey),
    /// List the keys and verify each decrypts; never prints a value
    Check(SecretsUnit),
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
