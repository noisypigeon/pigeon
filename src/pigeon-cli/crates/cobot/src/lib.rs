//! Infrastructure as code; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

pub mod apex;
pub mod catalog;
pub mod config;
pub mod generations;
pub mod moves;
pub mod providers;
pub mod release;
pub mod s3;
pub mod secrets;
pub mod sites;
pub mod static_site;
mod tofu;
pub mod unit;

use std::collections::BTreeMap;
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

/// The runner-supplied credentials, as JSON `{ name: { field: value } }`.
pub const CREDENTIALS_VAR: &str = "COBOT_CREDENTIALS";

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
    /// The workload secrets passphrase is missing or too short.
    WorkloadSecretsPassphrase,
    /// The two secrets passphrases are equal.
    SamePassphrase,
    /// An apply needs a handoff that state does not hold yet.
    Pending {
        /// The unit.
        unit: String,
        /// The token it waits for.
        token: String,
    },
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
            Self::WorkloadSecretsPassphrase => write!(
                f,
                "{} must be set, at least 16 characters, for a workload unit with secrets",
                secrets::WORKLOAD_PASSPHRASE_VAR
            ),
            Self::SamePassphrase => write!(
                f,
                "{} must differ from {}",
                secrets::WORKLOAD_PASSPHRASE_VAR,
                secrets::PASSPHRASE_VAR
            ),
            Self::Pending { unit, token } => write!(
                f,
                "{unit}: {} holds no live `{token}` yet; it applies after that token is minted",
                apex::CLOUDFLARE
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

/// The passphrase for the secrets of the unit at `unit_path`: the workload
/// passphrase under `workloads/`, the infra one elsewhere; the two must differ.
///
/// # Errors
///
/// [`Error::SecretsPassphrase`], [`Error::WorkloadSecretsPassphrase`] or [`Error::SamePassphrase`].
pub fn secrets_passphrase_for(
    unit_path: &str,
    infra: Option<String>,
    workload: Option<String>,
) -> Result<String, Error> {
    if infra.is_some() && infra == workload {
        return Err(Error::SamePassphrase);
    }
    if unit_path.trim_start_matches("./").starts_with("workloads/") {
        workload
            .filter(|p| p.chars().count() >= 16)
            .ok_or(Error::WorkloadSecretsPassphrase)
    } else {
        secrets_passphrase(infra)
    }
}

/// The passphrase for the unit at `unit_path` and its decrypted secrets from
/// `file`; a value that does not open names its key and variable, and says
/// when the other secrets passphrase opens it.
///
/// # Errors
///
/// As [`secrets_passphrase_for`], or [`Error::Io`] if a value cannot be decrypted.
pub fn unit_secrets(
    unit_path: &str,
    file: &Path,
    infra: Option<String>,
    workload: Option<String>,
) -> Result<(String, BTreeMap<String, String>), Error> {
    let pass = secrets_passphrase_for(unit_path, infra.clone(), workload.clone())?;
    let is_workload = unit_path.trim_start_matches("./").starts_with("workloads/");
    let (var, other_var, other) = if is_workload {
        (
            secrets::WORKLOAD_PASSPHRASE_VAR,
            secrets::PASSPHRASE_VAR,
            infra,
        )
    } else {
        (
            secrets::PASSPHRASE_VAR,
            secrets::WORKLOAD_PASSPHRASE_VAR,
            workload,
        )
    };
    let shown = Path::new(unit_path.trim_end_matches('/')).join(secrets::FILE);
    let values = secrets::decrypt_all_named(
        file,
        &shown,
        secrets::Named {
            var,
            passphrase: &pass,
        },
        other.as_deref().map(|passphrase| secrets::Named {
            var: other_var,
            passphrase,
        }),
    )?;
    Ok((pass, values))
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

/// The selected units, or all but the local ones.
///
/// # Errors
///
/// [`Error::Invalid`] if any unit is invalid, [`Error::UnknownUnit`] for an
/// unknown selection.
pub fn select(root: &Path, providers: &Path, wanted: &[String]) -> Result<Vec<Unit>, Error> {
    let units = validate(root, providers)?;
    let mut chosen: Vec<Unit> = if wanted.is_empty() {
        units.into_iter().filter(|u| !u.is_local()).collect()
    } else {
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
            .collect::<Result<_, _>>()?
    };
    chosen.sort_by_key(|u| !u.path.starts_with("infra/"));
    Ok(chosen)
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
    /// The secrets passphrase, needed only by infra units with secrets.
    pub secrets_passphrase: Option<String>,
    /// The workload secrets passphrase, needed only by workload units with secrets.
    pub workload_secrets_passphrase: Option<String>,
    /// Today, for token generations.
    pub today: generations::Day,
    /// The runner-supplied credentials, as JSON; needed only by units requesting some.
    pub credentials: Option<String>,
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
        return config::render(c, &state_file, allow, today, existing);
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
fn prepare(
    ctx: &Context,
    unit: &Unit,
    apply: bool,
) -> Result<(PathBuf, tofu::Env, UnitPlan), Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let state = absolute(&ctx.state)?;
    let allow = providers::Allowlist::load(&ctx.providers)?;
    let mut env = tofu::Env {
        passphrase: ctx.state_passphrase.clone(),
        lockfile: providers::lockfile(&ctx.providers),
        ..tofu::Env::default()
    };
    let mut pending = None;
    let moved = moved_state(&root, &state, unit)?;
    if let Some(c) = &unit.config
        && !config::declared_secrets(c).is_empty()
    {
        let (pass, values) = unit_secrets(
            &unit.path,
            &unit.dir.join(secrets::FILE),
            ctx.secrets_passphrase.clone(),
            ctx.workload_secrets_passphrase.clone(),
        )?;
        decrypted_records(unit, c, &values)?;
        env.hidden.extend(values.values().cloned());
        env.hidden.push(pass);
        let json = serde_json::to_string(&values).map_err(|e| Error::Io {
            path: unit.dir.join(secrets::FILE),
            message: e.to_string(),
        })?;
        env.vars.push(("TF_VAR_secrets".into(), json));
    }
    if let Some(c) = &unit.config {
        runner_credentials(ctx, unit, c, &mut env)?;
    }
    if let Some(c) = &unit.config
        && config::has_handoff(c)
        && let Some(site) = &c.site
    {
        let shown = infra_state(ctx, &root, &state, &allow)?.unwrap_or_else(|| "{}".into());
        pending = hand_off(ctx, unit, c, site, &shown, &state, apply, &mut env)?;
    }
    let work = root.join(WORK_DIR).join(&unit.path);
    let none = config::Existing::new();
    write_render(&work, &render_json(unit, &state, &allow, ctx.today, &none))?;
    let home = unit.config.as_ref().map_or(&unit.dir, |c| &c.home);
    env.providers = providers::used(&allow, home, &work.join("main.tf.json"))
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
    let mut plan = tofu::plan(&unit.path, &work, &env)?;
    plan.note = match (moved, pending) {
        (Some(m), Some(p)) => Some(format!("{m} {p}")),
        (m, p) => m.or(p),
    };
    Ok((work, env, plan))
}

/// Feeds the runner's credentials for the ones `unit` requests into `TF_VAR_credentials`;
/// see the phase 3 amendment of
/// `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.
fn runner_credentials(
    ctx: &Context,
    unit: &Unit,
    c: &config::ConfigUnit,
    env: &mut tofu::Env,
) -> Result<(), Error> {
    let wanted = config::credential_fields(c);
    if wanted.is_empty() {
        return Ok(());
    }
    let mine =
        pick_credentials(ctx.credentials.as_deref(), &wanted, &unit.path).map_err(|message| {
            Error::Io {
                path: PathBuf::from(CREDENTIALS_VAR),
                message,
            }
        })?;
    env.hidden
        .extend(mine.values().flat_map(|f| f.values().cloned()));
    let json = serde_json::to_string(&mine).map_err(|e| Error::Io {
        path: PathBuf::from(CREDENTIALS_VAR),
        message: e.to_string(),
    })?;
    env.vars.push(("TF_VAR_credentials".into(), json));
    Ok(())
}

/// From the runner's `json`, exactly the fields `unit` references of each credential it
/// requests (`wanted`); other credentials and fields are never read. Never echoes a value.
///
/// # Errors
///
/// A message when `json` is not an object, or a wanted credential or field is missing or not
/// a string.
pub fn pick_credentials(
    json: Option<&str>,
    wanted: &BTreeMap<String, std::collections::BTreeSet<String>>,
    unit: &str,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let all: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(json.unwrap_or("{}"))
            .map_err(|_| "must be a JSON object mapping credential names to fields".to_string())?;
    let mut out = BTreeMap::new();
    for (name, fields) in wanted {
        let entry = all.get(name).filter(|e| e.is_object()).ok_or_else(|| {
            format!(
                "{unit} requests credential `{name}`, which the runner did not supply as an object"
            )
        })?;
        let mut picked = BTreeMap::new();
        for field in fields {
            let value = entry
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "{unit} needs `{name}.{field}`, which the runner did not supply as a string"
                    )
                })?;
            picked.insert(field.clone(), value.to_string());
        }
        out.insert(name.clone(), picked);
    }
    Ok(out)
}

/// Copies a moved unit's state into place while it has none; the plan's note, if moving.
fn moved_state(root: &Path, state: &Path, unit: &Unit) -> Result<Option<String>, Error> {
    let moves = moves::load(root).map_err(|message| Error::Io {
        path: root.join(moves::FILE),
        message,
    })?;
    let Some(m) = moves::into(&moves, &unit.path) else {
        return Ok(None);
    };
    let from = state.join(&m.from).join(moves::STATE);
    let to = state.join(&m.to).join(moves::STATE);
    if !from.exists() {
        return Ok(None);
    }
    if !to.exists() {
        let dir = state.join(&m.to);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        fs::copy(&from, &to).map_err(io(&to))?;
    }
    Ok(Some(format!(
        "State moves from `{}`; it is removed after this unit's next successful apply.",
        m.from
    )))
}

/// After a successful apply, removes the state a unit moved from.
fn finish_move(ctx: &Context, unit: &Unit) -> Result<(), Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let moves = moves::load(&root).map_err(|message| Error::Io {
        path: root.join(moves::FILE),
        message,
    })?;
    if let Some(m) = moves::into(&moves, &unit.path) {
        let from = absolute(&ctx.state)?.join(&m.from).join(moves::STATE);
        let to = absolute(&ctx.state)?.join(&m.to).join(moves::STATE);
        if from.exists() && to.exists() {
            fs::remove_file(&from).map_err(io(&from))?;
        }
    }
    Ok(())
}

/// Rejects an apex whose secret-backed records conflict once decrypted.
fn decrypted_records(
    unit: &Unit,
    c: &config::ConfigUnit,
    values: &std::collections::BTreeMap<String, String>,
) -> Result<(), Error> {
    let Some(site) = &c.site else {
        return Ok(());
    };
    let found = apex::check_decrypted(&c.config, &c.home, site, values);
    if found.is_empty() {
        return Ok(());
    }
    Err(Error::Invalid(
        found
            .into_iter()
            .map(|message| Problem {
                path: format!("{}/{}", unit.path, config::CONFIG),
                message,
            })
            .collect(),
    ))
}

/// Sets `TF_VAR_handoff` for an apex or a static site from `infra/cloudflare`'s
/// state; while a token is missing, plans with placeholders and returns the
/// note, and an apply refuses.
#[expect(
    clippy::too_many_arguments,
    reason = "one unit's handoff, from prepare"
)]
fn hand_off(
    ctx: &Context,
    unit: &Unit,
    c: &config::ConfigUnit,
    site: &apex::Site,
    shown: &str,
    state: &Path,
    apply: bool,
    env: &mut tofu::Env,
) -> Result<Option<String>, Error> {
    let (json, values, waiting) = if let Some(w) = &site.workload {
        let mut v = static_site::handoff(shown, &site.zone, w, ctx.today);
        let waiting: Vec<String> = (!static_site::HANDOFF_KEYS.iter().all(|k| v.contains_key(*k)))
            .then(|| static_site::token_name(&site.zone, w))
            .into_iter()
            .collect();
        if !waiting.is_empty() {
            placeholders(&mut v, &static_site::HANDOFF_KEYS);
        }
        (
            serde_json::to_string(&v),
            v.into_values().collect::<Vec<_>>(),
            waiting,
        )
    } else {
        let mut values = apex::handoff(shown, &apex::zones(&c.config), ctx.today);
        let waiting: Vec<String> = values
            .iter()
            .filter(|(_, v)| !apex::HANDOFF_KEYS.iter().all(|k| v.contains_key(*k)))
            .map(|(z, _)| apex::token_name(z))
            .collect();
        if !waiting.is_empty() {
            for v in values.values_mut() {
                placeholders(v, &apex::HANDOFF_KEYS);
            }
        }
        (
            serde_json::to_string(&values),
            values
                .into_values()
                .flat_map(BTreeMap::into_values)
                .collect(),
            waiting,
        )
    };
    let mut pending = None;
    if !waiting.is_empty() {
        let tokens = waiting.join("`, `");
        if apply {
            return Err(Error::Pending {
                unit: unit.path.clone(),
                token: tokens,
            });
        }
        let existing = state.join(&unit.path).join("terraform.tfstate").exists();
        env.no_refresh = existing;
        pending = Some(format!(
            "Credentials pending: planned with placeholders{} until {} mints `{tokens}`.",
            if existing { ", refresh skipped," } else { "" },
            apex::CLOUDFLARE
        ));
    }
    env.hidden.extend(values);
    let json = json.map_err(|e| Error::Io {
        path: unit.dir.clone(),
        message: e.to_string(),
    })?;
    env.vars.push(("TF_VAR_handoff".into(), json));
    Ok(pending)
}

/// Fills the `keys` a pending plan lacks with inert placeholders.
fn placeholders(values: &mut BTreeMap<String, String>, keys: &[&str]) {
    for k in keys {
        let inert = match *k {
            "zone_id" => apex::PLACEHOLDER_ZONE_ID,
            "account_id" => static_site::PLACEHOLDER_ACCOUNT_ID,
            _ => apex::PLACEHOLDER_TOKEN,
        };
        values.entry((*k).into()).or_insert_with(|| inert.into());
    }
}

/// `infra/cloudflare`'s state as `tofu show -json`, if it has any.
fn infra_state(
    ctx: &Context,
    root: &Path,
    state: &Path,
    allow: &providers::Allowlist,
) -> Result<Option<String>, Error> {
    if !state
        .join(apex::CLOUDFLARE)
        .join("terraform.tfstate")
        .exists()
    {
        return Ok(None);
    }
    let Some(source) = unit::discover(root)
        .0
        .into_iter()
        .find(|u| u.path == apex::CLOUDFLARE)
    else {
        return Ok(None);
    };
    let work = root.join(WORK_DIR).join(&source.path);
    let rendered = render_json(&source, state, allow, ctx.today, &config::Existing::new());
    write_render(&work, &rendered)?;
    let mut env = tofu::Env {
        passphrase: ctx.state_passphrase.clone(),
        lockfile: providers::lockfile(&ctx.providers),
        ..tofu::Env::default()
    };
    env.providers = providers::used(allow, &source.dir, &work.join("main.tf.json"))
        .into_iter()
        .map(str::to_string)
        .collect();
    tofu::init(&source.path, &work, &env)?;
    tofu::show_state(&source.path, &work, &env).map(Some)
}

/// One published static site.
#[derive(Debug, Clone)]
pub struct Published {
    /// The unit path; its bucket is never reported.
    pub unit: String,
    /// What the sync did.
    pub synced: sites::Synced,
    /// The live checks.
    pub checks: Vec<sites::Check>,
}

/// Builds each static site, syncs it to its bucket with its site token, then
/// runs the live checks; `checks` of 0 skips them.
///
/// # Errors
///
/// [`Error::Pending`] while `infra/cloudflare`'s state lacks a site's token,
/// or any [`Error`] from building or syncing; never a credential.
pub fn publish(ctx: &Context, wanted: &[String], checks: u32) -> Result<Vec<Published>, Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let state = absolute(&ctx.state)?;
    let allow = providers::Allowlist::load(&ctx.providers)?;
    let found = sites::find(&root, wanted)?;
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let built = sites::build(&root, &found)?;
    let shown = infra_state(ctx, &root, &state, &allow)?.unwrap_or_else(|| "{}".into());
    let mut out = Vec::new();
    for b in built {
        let Some((creds, account)) = sites::credentials(&shown, &b.site, ctx.today) else {
            return Err(Error::Pending {
                unit: b.site.unit.clone(),
                token: static_site::token_name(&b.site.zone, &b.site.workload),
            });
        };
        let bucket = s3::Bucket {
            endpoint: sites::endpoint(&account),
            name: b.site.bucket(),
            creds,
        };
        let expected = sites::expected(&b.public)?;
        let synced = sites::sync(&bucket, &expected).map_err(|e| match e {
            Error::Tool { message, .. } => Error::Tool {
                command: format!("publish {}", b.site.unit),
                message,
            },
            other => other,
        })?;
        let checks = sites::checks(&b, &expected, checks, std::time::Duration::from_secs(15));
        out.push(Published {
            unit: b.site.unit.clone(),
            synced,
            checks,
        });
    }
    Ok(out)
}

/// Plans each unit.
///
/// # Errors
///
/// Any [`Error`]; `tofu` failures are redacted.
pub fn plan(ctx: &Context, units: &[Unit]) -> Result<Vec<UnitPlan>, Error> {
    units
        .iter()
        .map(|u| prepare(ctx, u, false).map(|(_, _, p)| p))
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
        let (work, env, p) = prepare(ctx, u, true)?;
        if p.has_changes() {
            tofu::apply(&u.path, &work, &env)?;
        }
        finish_move(ctx, u)?;
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
        if let Some(note) = &p.note {
            let _ = write!(out, "{note}\n\n");
        }
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
