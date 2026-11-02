//! Infrastructure as code; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

pub mod config;
pub mod generations;
pub mod providers;
pub mod release;
pub mod secrets;
mod tofu;
pub mod unit;

use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub use tofu::{Action, Change, UnitPlan};
pub use unit::{Problem, Unit};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The work directory under the root, gitignored.
pub const WORK_DIR: &str = ".cobot";

/// The environment variable holding the state passphrase.
pub const PASSPHRASE_VAR: &str = "COBOT_STATE_PASSPHRASE";

/// Why a command could not run.
#[derive(Debug)]
pub enum Error {
    /// Units failed validation.
    Invalid(Vec<Problem>),
    /// A requested unit does not exist.
    UnknownUnit(String),
    /// The state passphrase is missing or too short.
    Passphrase,
    /// The secrets passphrase is missing or too short.
    SecretsPassphrase,
    /// A file could not be read, written or parsed.
    Io {
        /// The path.
        path: PathBuf,
        /// Why.
        message: String,
    },
    /// `tofu` failed.
    Tofu {
        /// The unit.
        unit: String,
        /// The step and its diagnostic summaries, redacted.
        message: String,
    },
    /// A git command failed.
    Git {
        /// The arguments.
        args: String,
        /// Its stderr, or why it could not start.
        message: String,
    },
    /// Another program failed.
    Tool {
        /// The command line.
        command: String,
        /// Its stderr, or why it could not start.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problems) => {
                write!(f, "{} problem(s):", problems.len())?;
                for p in problems {
                    write!(f, "\n{}: {}", p.path, p.message)?;
                }
                Ok(())
            }
            Self::UnknownUnit(u) => write!(f, "no unit at `{u}`"),
            Self::Passphrase => write!(
                f,
                "{PASSPHRASE_VAR} must be set, at least 16 characters; cobot never writes plaintext state"
            ),
            Self::SecretsPassphrase => write!(
                f,
                "{} must be set, at least 16 characters, for a unit with secrets",
                secrets::PASSPHRASE_VAR
            ),
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Tofu { unit, message } => write!(f, "{unit}: {message}"),
            Self::Git { args, message } => write!(f, "git {args}: {message}"),
            Self::Tool { command, message } => write!(f, "{command}: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// The state passphrase, checked.
///
/// # Errors
///
/// [`Error::Passphrase`] if `value` is missing or shorter than 16 characters.
pub fn passphrase(value: Option<String>) -> Result<String, Error> {
    value
        .filter(|p| p.chars().count() >= 16)
        .ok_or(Error::Passphrase)
}

/// A secrets passphrase, checked.
///
/// # Errors
///
/// [`Error::SecretsPassphrase`] if `value` is missing or shorter than 16 characters.
pub fn secrets_passphrase(value: Option<String>) -> Result<String, Error> {
    value
        .filter(|p| p.chars().count() >= 16)
        .ok_or(Error::SecretsPassphrase)
}

/// Validates every unit, against the provider allowlist in `providers`.
///
/// # Errors
///
/// [`Error::Invalid`] listing every problem; [`Error::Io`] for an unreadable allowlist.
pub fn validate(root: &Path, providers: &Path) -> Result<Vec<Unit>, Error> {
    let root = &root.canonicalize().map_err(io(root))?;
    let allow = providers::Allowlist::load(providers)?;
    let problems = unit::validate(root, &allow);
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
    }
    Ok(unit::discover(root).0)
}

/// The selected units, or all of them.
///
/// # Errors
///
/// [`Error::Invalid`] if any unit is invalid, [`Error::UnknownUnit`] for an
/// unknown selection.
pub fn select(root: &Path, providers: &Path, wanted: &[String]) -> Result<Vec<Unit>, Error> {
    let units = validate(root, providers)?;
    if wanted.is_empty() {
        return Ok(units);
    }
    wanted
        .iter()
        .map(|w| {
            let w = w.trim_end_matches('/');
            units
                .iter()
                .find(|u| u.path == w)
                .cloned()
                .ok_or_else(|| Error::UnknownUnit(w.to_string()))
        })
        .collect()
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Everything a render, plan or apply needs beyond the units.
#[derive(Debug, Clone)]
pub struct Context {
    /// The repository root.
    pub root: PathBuf,
    /// The directory holding `providers.yaml` and `providers.lock.hcl`.
    pub providers: PathBuf,
    /// The state repository's checkout.
    pub state: PathBuf,
    /// The state passphrase.
    pub state_passphrase: String,
    /// The secrets passphrase, needed only by units with secrets.
    pub secrets_passphrase: Option<String>,
    /// Today, for token generations.
    pub today: generations::Day,
}

/// The rendered `main.tf.json` for `unit`.
#[must_use]
pub fn render_json(
    unit: &Unit,
    state: &Path,
    allow: &providers::Allowlist,
    today: generations::Day,
    existing: &config::Existing,
) -> Value {
    let state_file = state.join(&unit.path).join("terraform.tfstate");
    if let Some(c) = &unit.config {
        return config::render(c, &unit.dir, &state_file, allow, today, existing);
    }
    let mut modules = Map::new();
    if let Some(spec) = &unit.spec {
        for m in &spec.modules {
            let mut block = Map::new();
            block.insert(
                "source".into(),
                Value::String(unit.dir.join(&m.source).display().to_string()),
            );
            for (k, v) in &m.inputs {
                block.insert(k.clone(), v.clone());
            }
            modules.insert(m.name.clone(), Value::Object(block));
        }
    }
    json!({
        "terraform": {
            "required_version": ">= 1.13.0",
            "backend": { "local": { "path": state_file.display().to_string() } }
        },
        "module": modules
    })
}

fn write_render(work: &Path, value: &Value) -> Result<(), Error> {
    fs::create_dir_all(work).map_err(io(work))?;
    let file = work.join("main.tf.json");
    let text = serde_json::to_string_pretty(value).map_err(|e| Error::Io {
        path: file.clone(),
        message: e.to_string(),
    })?;
    fs::write(&file, text + "\n").map_err(io(&file))
}

/// Writes `<root>/.cobot/<unit>/main.tf.json`, with no retained generations; returns the work directory.
///
/// # Errors
///
/// [`Error::Io`] if the work directory cannot be written.
pub fn render(root: &Path, providers: &Path, unit: &Unit, state: &Path) -> Result<PathBuf, Error> {
    let root = &root.canonicalize().map_err(io(root))?;
    let state = &absolute(state)?;
    let allow = providers::Allowlist::load(providers)?;
    let work = root.join(WORK_DIR).join(&unit.path);
    let value = render_json(
        unit,
        state,
        &allow,
        generations::today(),
        &config::Existing::new(),
    );
    write_render(&work, &value)?;
    Ok(work)
}

fn absolute(path: &Path) -> Result<PathBuf, Error> {
    fs::create_dir_all(path).map_err(io(path))?;
    path.canonicalize().map_err(io(path))
}

/// Renders, initializes and plans one unit, retaining live generations from state.
fn prepare(ctx: &Context, unit: &Unit) -> Result<(PathBuf, tofu::Env, UnitPlan), Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let state = absolute(&ctx.state)?;
    let allow = providers::Allowlist::load(&ctx.providers)?;
    let mut env = tofu::Env {
        passphrase: ctx.state_passphrase.clone(),
        lockfile: providers::lockfile(&ctx.providers),
        ..tofu::Env::default()
    };
    if let Some(c) = &unit.config
        && !config::secret_keys(&c.schema).is_empty()
    {
        let pass = &secrets_passphrase(ctx.secrets_passphrase.clone())?;
        let values = secrets::decrypt_all(&unit.dir.join(secrets::FILE), pass)?;
        env.hidden.extend(values.values().cloned());
        env.hidden.push(pass.clone());
        let json = serde_json::to_string(&values).map_err(|e| Error::Io {
            path: unit.dir.join(secrets::FILE),
            message: e.to_string(),
        })?;
        env.vars.push(("TF_VAR_secrets".into(), json));
    }
    let work = root.join(WORK_DIR).join(&unit.path);
    let none = config::Existing::new();
    write_render(&work, &render_json(unit, &state, &allow, ctx.today, &none))?;
    env.providers = providers::used(&allow, &unit.dir, &work.join("main.tf.json"))
        .into_iter()
        .map(str::to_string)
        .collect();
    tofu::init(&unit.path, &work, &env)?;
    if let Some(c) = &unit.config {
        let rotating = config::rotating_blocks(&c.schema);
        if !rotating.is_empty() && state.join(&unit.path).join("terraform.tfstate").exists() {
            let existing = config::existing(&tofu::show_state(&unit.path, &work, &env)?, &rotating);
            write_render(
                &work,
                &render_json(unit, &state, &allow, ctx.today, &existing),
            )?;
        }
    }
    let plan = tofu::plan(&unit.path, &work, &env)?;
    Ok((work, env, plan))
}

/// Plans each unit.
///
/// # Errors
///
/// Any [`Error`]; `tofu` failures are redacted.
pub fn plan(ctx: &Context, units: &[Unit]) -> Result<Vec<UnitPlan>, Error> {
    units
        .iter()
        .map(|u| prepare(ctx, u).map(|(_, _, p)| p))
        .collect()
}

/// Plans, then applies each unit with changes.
///
/// # Errors
///
/// Any [`Error`]; `tofu` failures are redacted.
pub fn apply(ctx: &Context, units: &[Unit]) -> Result<Vec<UnitPlan>, Error> {
    let mut plans = Vec::new();
    for u in units {
        let (work, env, p) = prepare(ctx, u)?;
        if p.has_changes() {
            tofu::apply(&u.path, &work, &env)?;
        }
        plans.push(p);
    }
    Ok(plans)
}

/// Writes `<providers>/providers.lock.hcl` for every allowlisted provider,
/// on every platform cobot runs on; returns its path.
///
/// # Errors
///
/// [`Error::Io`] or [`Error::Tofu`] if the scratch configuration or `tofu` fails.
pub fn lock(root: &Path, providers: &Path) -> Result<PathBuf, Error> {
    let root = root.canonicalize().map_err(io(root))?;
    let allow = providers::Allowlist::load(providers)?;
    let work = root.join(WORK_DIR).join("lock");
    let mut required = Map::new();
    for (source, version) in &allow.providers {
        required.insert(
            providers::Allowlist::local_name(source).to_string(),
            json!({ "source": source, "version": version }),
        );
    }
    write_render(
        &work,
        &json!({ "terraform": { "required_providers": required } }),
    )?;
    let output = std::process::Command::new("tofu")
        .args([
            "providers",
            "lock",
            "-platform=linux_amd64",
            "-platform=linux_arm64",
            "-platform=darwin_arm64",
        ])
        .current_dir(&work)
        .output()
        .map_err(io(&work))?;
    if !output.status.success() {
        return Err(Error::Tofu {
            unit: "lock".into(),
            message: String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|l| l.trim_start().starts_with("Error:"))
                .collect::<Vec<_>>()
                .join("; "),
        });
    }
    let target = providers.join(providers::LOCKFILE);
    fs::copy(work.join(".terraform.lock.hcl"), &target).map_err(io(&target))?;
    Ok(target)
}

/// A Markdown summary of `plans`: addresses and actions only.
#[must_use]
pub fn summary(plans: &[UnitPlan]) -> String {
    let mut out = String::from("<!-- cobot-plan -->\n## cobot plan\n");
    for p in plans {
        let _ = write!(out, "\n### `{}`\n\n", p.unit);
        if p.has_changes() {
            for c in &p.changes {
                let _ = writeln!(out, "- {} `{}`", c.action.word(), c.address);
            }
            let (add, change, destroy) = p.counts();
            let _ = write!(
                out,
                "\n{add} to add, {change} to change, {destroy} to destroy.\n"
            );
        } else {
            out.push_str("No changes.\n");
        }
    }
    out
}
