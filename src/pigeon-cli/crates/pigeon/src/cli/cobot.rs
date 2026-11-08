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
    /// Print a stored plan's changes attribute by attribute; sensitive values stay hidden
    Show(CobotShow),
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
    /// Host the instance's git repositories on Cloudflare Artifacts, mirrored to GitHub
    #[command(subcommand)]
    Git(GitCommand),
    /// Render, plan and deploy the platform's Workers with wrangler
    #[command(subcommand)]
    Workers(WorkersCommand),
    /// Print the checks a change needs, from the paths it touches; with --run, run them
    Scope(ScopeArgs),
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
    /// The instance whose units to use, when not the repository root; catalogs, providers and
    /// templates still come from the root
    #[arg(long)]
    pub instance: Option<PathBuf>,
}

impl CobotRoot {
    /// The instance's root: `--instance`, else the repository root.
    #[must_use]
    pub fn instance_root(&self) -> &std::path::Path {
        self.instance.as_deref().unwrap_or(&self.root)
    }

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

/// `pigeon cobot show` arguments.
#[derive(Args, Debug)]
pub struct CobotShow {
    /// The state holding the plan
    #[command(flatten)]
    pub run: CobotRun,
    /// The stored plan, from `plan --store`
    #[arg(long)]
    pub plan: String,
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

/// `pigeon cobot git` commands; each uses `CLOUDFLARE_API_TOKEN`, scoped to the instance's own
/// account.
#[derive(Subcommand, Debug)]
pub enum GitCommand {
    /// Create the instance's namespace and repositories, importing new ones from their mirror
    Apply(CobotRoot),
    /// Print each repository's remote and its head on Artifacts and on its mirror
    Status(CobotRoot),
    /// `git push <remote> [ARGS…]` with a short-lived write token
    Push(GitRun),
    /// `git fetch <remote> [ARGS…]` with a short-lived read token
    Fetch(GitRun),
    /// `git clone <remote> [ARGS…]` with a short-lived read token
    Clone(GitRun),
    /// Push each repository's default branch and tags to its mirror, with your own git credentials
    Mirror(GitMirror),
    /// Bring each repository up to its mirror's release commits and tags
    Sync(GitMirror),
}

/// A repository and the git arguments after its remote.
#[derive(Args, Debug)]
pub struct GitRun {
    /// The repository root and instance
    #[command(flatten)]
    pub at: CobotRoot,
    /// The repository, e.g. pigeon
    pub repo: String,
    /// Arguments after the remote, e.g. HEAD:main
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// `pigeon cobot workers` commands; see
/// `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.
#[derive(Subcommand, Debug)]
pub enum WorkersCommand {
    /// Check every spec; exit 1 on a problem
    Validate(WorkersAt),
    /// Print the token permissions the specs need, on the account
    Permissions(WorkersAt),
    /// Write each Worker's wrangler.jsonc under .cobot/workers/, its references resolved from state
    Render(WorkersState),
    /// Render, and store a plan for `deploy --plan`, valid for 24 hours
    Plan(WorkersState),
    /// Deploy a stored, unexpired plan: D1 migrations, images, then each Worker
    Deploy(WorkersDeploy),
    /// Print each deployed Worker's status
    Status(WorkersAt),
    /// Set a Worker secret its spec declares, from stdin
    #[command(subcommand)]
    Secrets(WorkersSecrets),
    /// Delete a Worker and its Container applications
    Teardown(WorkersOne),
    /// Print the state's outputs the specs reference, as JSON, for
    /// `secrets put <worker> DEPLOY_OUTPUTS`
    Outputs(WorkersState),
    /// Container images in Cloudflare's registry
    #[command(subcommand)]
    Images(WorkersImages),
}

/// `pigeon cobot workers images` commands.
#[derive(Subcommand, Debug)]
pub enum WorkersImages {
    /// Build each container's image with Docker and push it to Cloudflare's registry, logged in
    /// for as long as the push takes; print their references as JSON, for
    /// `secrets put <worker> DEPLOY_IMAGES`
    Push(ImagesPush),
}

/// `pigeon cobot workers images push` arguments.
#[derive(Args, Debug)]
pub struct ImagesPush {
    /// Workers, and the root
    #[command(flatten)]
    pub at: WorkersAt,
    /// How long the registry login lasts, in minutes
    #[arg(long, default_value_t = cobot::workers::LOGIN_MINUTES)]
    pub expiration_minutes: u32,
}

/// The repository root, and Workers by name [default: every one].
#[derive(Args, Debug)]
pub struct WorkersAt {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The stack a spec with `deployment:` is bound to, by its unit ID, e.g. `platform/blue`
    #[arg(long)]
    pub deployment: Option<String>,
    /// Workers, by name
    pub names: Vec<String>,
}

/// [`WorkersAt`] and the state the specs' references resolve from.
#[derive(Args, Debug)]
pub struct WorkersState {
    /// Workers, and the root
    #[command(flatten)]
    pub at: WorkersAt,
    /// The state, as for `pigeon cobot plan`
    #[arg(long)]
    pub state: PathBuf,
    /// The directory holding providers.yaml and providers.lock.hcl [default: ./.config]
    #[arg(long)]
    pub providers: Option<PathBuf>,
}

/// `pigeon cobot workers deploy` arguments.
#[derive(Args, Debug)]
pub struct WorkersDeploy {
    /// The state, and the root
    #[command(flatten)]
    pub run: WorkersState,
    /// The stored plan
    #[arg(long)]
    pub plan: String,
}

/// One Worker.
#[derive(Args, Debug)]
pub struct WorkersOne {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The stack a spec with `deployment:` is bound to, by its unit ID
    #[arg(long)]
    pub deployment: Option<String>,
    /// The Worker
    pub name: String,
}

/// `pigeon cobot workers secrets` commands; there is deliberately no `get`.
#[derive(Subcommand, Debug)]
pub enum WorkersSecrets {
    /// Set KEY on a Worker from stdin
    Put(WorkersSecret),
}

/// `pigeon cobot workers secrets put` arguments.
#[derive(Args, Debug)]
pub struct WorkersSecret {
    /// The Worker
    #[command(flatten)]
    pub worker: WorkersOne,
    /// The secret's name, as its spec declares it
    pub key: String,
}

/// `pigeon cobot scope` arguments.
#[derive(Args, Debug)]
pub struct ScopeArgs {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The change's base; its paths are those changed since, plus uncommitted ones
    #[arg(long, default_value = "origin/main")]
    pub base: String,
    /// The paths instead, one per --file; the base is then not read
    #[arg(long = "file")]
    pub files: Vec<String>,
    /// Run the checks, in order, and exit 1 if one fails
    #[arg(long)]
    pub run: bool,
    /// Print the checks as JSON
    #[arg(long, conflicts_with = "run")]
    pub json: bool,
}

/// `pigeon cobot git mirror` arguments.
#[derive(Args, Debug)]
pub struct GitMirror {
    /// The repository root and instance
    #[command(flatten)]
    pub at: CobotRoot,
    /// Repositories [default: every one with a mirror]
    pub repos: Vec<String>,
}

/// `pigeon cobot sites build` arguments.
#[derive(Args, Debug)]
pub struct SitesBuild {
    /// The repository root
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// The instance whose sites to build, when not the repository root; templates still
    /// come from the root
    #[arg(long)]
    pub instance: Option<PathBuf>,
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
