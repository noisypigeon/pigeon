//! The `engines` build: everything that runs `tofu`, `zola` or `git`, or touches files and R2
//! (control-plane ADR §1).

use crate::{
    apex, cloudflare, config, generations, moves, plans, providers, s3, secrets, sites,
    static_site, tofu, unit,
};

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub use crate::tofu::{Action, Change, UnitPlan};
pub use crate::unit::{Problem, Unit};

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
    /// The R2 state store failed, or another writer changed state since it was read.
    State(String),
    /// A stored plan is unknown, expired, or no longer matches its units.
    Plan(String),
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
            Self::State(message) => write!(f, "state: {message}"),
            Self::Plan(message) => write!(f, "plan: {message}"),
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
    validate_in(root, root, providers)
}

/// [`validate`] for the instance at `root` on the platform at `platform`.
///
/// # Errors
///
/// As [`validate`].
pub fn validate_in(root: &Path, platform: &Path, providers: &Path) -> Result<Vec<Unit>, Error> {
    let root = &root.canonicalize().map_err(io(root))?;
    let platform = &platform.canonicalize().map_err(io(platform))?;
    let allow = providers::Allowlist::load(providers)?;
    let problems = unit::validate_in(root, platform, &allow);
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
    }
    Ok(unit::discover_in(root, platform).0)
}

/// The selected units, or all but the local ones.
///
/// # Errors
///
/// [`Error::Invalid`] if any unit is invalid, [`Error::UnknownUnit`] for an
/// unknown selection.
pub fn select(root: &Path, providers: &Path, wanted: &[String]) -> Result<Vec<Unit>, Error> {
    select_in(root, root, providers, wanted)
}

/// [`select`] for the instance at `root` on the platform at `platform`.
///
/// # Errors
///
/// As [`select`].
pub fn select_in(
    root: &Path,
    platform: &Path,
    providers: &Path,
    wanted: &[String],
) -> Result<Vec<Unit>, Error> {
    let units = validate_in(root, platform, providers)?;
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
#[derive(Clone)]
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
    /// The run's own Cloudflare token, for workloads that use the catalogs directly; see
    /// `.claude/decisions/2026-11-05-development-pigeon-its-own-instance.md` §4.
    pub cloudflare_token: Option<String>,
    /// The Cloudflare API's base URL; `None` is [`cloudflare::API`].
    pub cloudflare_api: Option<String>,
    /// The platform's root, for catalogs and site templates, when the instance at `root` is
    /// elsewhere; see `.claude/decisions/2026-11-05-development-pigeon-its-own-instance.md`
    /// (2026-11-05 amendment).
    pub platform: Option<PathBuf>,
    /// Whether a plan adopts the resources its account already holds under the names a unit
    /// declares, through `import` blocks; see [`crate::adopt`].
    pub adopt: bool,
}

/// Shows which secrets are set, never their values.
impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = |v: &Option<String>| if v.is_some() { "set" } else { "unset" };
        f.debug_struct("Context")
            .field("root", &self.root)
            .field("providers", &self.providers)
            .field("state", &self.state)
            .field("state_passphrase", &"set")
            .field("secrets_passphrase", &set(&self.secrets_passphrase))
            .field(
                "workload_secrets_passphrase",
                &set(&self.workload_secrets_passphrase),
            )
            .field("today", &self.today)
            .field("credentials", &set(&self.credentials))
            .field("cloudflare_token", &set(&self.cloudflare_token))
            .field("cloudflare_api", &self.cloudflare_api)
            .field("platform", &self.platform)
            .field("adopt", &self.adopt)
            .finish()
    }
}

impl Context {
    /// The platform's root: [`Context::platform`]'s, else the instance's.
    #[must_use]
    pub fn platform_root(&self) -> &Path {
        self.platform.as_deref().unwrap_or(&self.root)
    }

    /// The API with the run's own token, if it has one.
    #[must_use]
    pub fn cloudflare(&self) -> Option<cloudflare::Api> {
        self.cloudflare_token
            .clone()
            .map(|t| cloudflare::Api::new(self.cloudflare_api.as_deref(), t))
    }
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
    let (work, env, note) = ready(ctx, unit, apply)?;
    let mut plan = tofu::plan(&unit.path, &work, &env)?;
    plan.note = note;
    Ok((work, env, plan))
}

/// Renders and initializes one unit, retaining live generations from state; its work
/// directory, environment, and the plan's note.
fn ready(
    ctx: &Context,
    unit: &Unit,
    apply: bool,
) -> Result<(PathBuf, tofu::Env, Option<String>), Error> {
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
        if let Some(api) = ctx.cloudflare() {
            run_hand_off(&api, c, site, &mut env)?;
        } else {
            let shown = infra_state(ctx, &root, &state, &allow)?.unwrap_or_else(|| "{}".into());
            pending = hand_off(ctx, unit, c, site, &shown, &state, apply, &mut env)?;
        }
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
    if ctx.adopt && unit.config.is_some() {
        adopt_into(ctx, &work)?;
    }
    let remark = match (moved, pending) {
        (Some(m), Some(p)) => Some(format!("{m} {p}")),
        (m, p) => m.or(p),
    };
    Ok((work, env, remark))
}

/// Adds to `work`'s `main.tf.json` an `import` block for each resource its account already
/// holds under a declared name.
fn adopt_into(ctx: &Context, work: &Path) -> Result<(), Error> {
    let file = work.join("main.tf.json");
    let failed = |message: String| Error::Io {
        path: file.clone(),
        message: format!("adopting by name: {message}"),
    };
    let api = ctx
        .cloudflare()
        .ok_or_else(|| failed("needs the run's Cloudflare token".into()))?;
    let account = api.account().map_err(failed)?;
    let mut value: Value = fs::read_to_string(&file)
        .map_err(io(&file))
        .and_then(|t| serde_json::from_str(&t).map_err(|e| failed(e.to_string())))?;
    let lookup = crate::adopt::Cloudflare {
        api: &api,
        account: &account,
    };
    let imports = crate::adopt::imports(&value, &account, &lookup).map_err(failed)?;
    if !imports.is_empty() {
        value["import"] = Value::Array(imports);
        write_render(work, &value)?;
    }
    Ok(())
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
    let filled = fill_credentials(
        ctx.credentials.as_deref(),
        &config::credential_providers(c),
        ctx.cloudflare_token.as_deref(),
    );
    let mine =
        pick_credentials(filled.as_deref(), &wanted, &unit.path).map_err(|message| Error::Io {
            path: PathBuf::from(CREDENTIALS_VAR),
            message,
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

/// The runner's `json` with each requested `cloudflare` credential it does not name filled
/// from the run's own `token`, per
/// `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md` §3; `json` itself when
/// there is nothing to fill or it is not an object.
#[must_use]
pub fn fill_credentials(
    json: Option<&str>,
    providers: &BTreeMap<String, String>,
    token: Option<&str>,
) -> Option<String> {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return json.map(str::to_string);
    };
    let Ok(mut all) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json.unwrap_or("{}"))
    else {
        return json.map(str::to_string);
    };
    for (name, provider) in providers {
        if provider == "cloudflare" && !all.contains_key(name) {
            all.insert(name.clone(), json!({ "api_token": token }));
        }
    }
    Some(serde_json::Value::Object(all).to_string())
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

/// Sets `TF_VAR_handoff` for an apex or a static site from the run's own token: the zone IDs
/// and account it sees, and the token itself.
fn run_hand_off(
    api: &cloudflare::Api,
    c: &config::ConfigUnit,
    site: &apex::Site,
    env: &mut tofu::Env,
) -> Result<(), Error> {
    let fail = |message: String| Error::Tool {
        command: "cloudflare".into(),
        message,
    };
    let json = if site.workload.is_some() {
        let v = BTreeMap::from([
            ("zone_id", api.zone_id(&site.zone).map_err(fail)?),
            ("account_id", api.account().map_err(fail)?),
            ("api_token", api.token().to_string()),
        ]);
        serde_json::to_string(&v)
    } else {
        let mut values = BTreeMap::new();
        for zone in apex::zones(&c.config) {
            let v = BTreeMap::from([
                ("zone_id", api.zone_id(&zone).map_err(fail)?),
                ("api_token", api.token().to_string()),
            ]);
            values.insert(zone, v);
        }
        serde_json::to_string(&values)
    };
    env.hidden.push(api.token().to_string());
    let json = json.map_err(|e| fail(e.to_string()))?;
    env.vars.push(("TF_VAR_handoff".into(), json));
    Ok(())
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
    unit_state(ctx, root, state, allow, apex::CLOUDFLARE)
}

/// Unit `path`'s state as `tofu show -json`, if it has any.
fn unit_state(
    ctx: &Context,
    root: &Path,
    state: &Path,
    allow: &providers::Allowlist,
    path: &str,
) -> Result<Option<String>, Error> {
    if !state.join(path).join("terraform.tfstate").exists() {
        return Ok(None);
    }
    let Some(source) = unit::discover_in(root, ctx.platform_root())
        .0
        .into_iter()
        .find(|u| u.path == path)
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

/// Unit `path`'s outputs, each name to its value, from its state; empty until it has state.
///
/// # Errors
///
/// [`Error::Tool`] or [`Error::Io`] if its state can't be read.
pub fn outputs(ctx: &Context, path: &str) -> Result<serde_json::Map<String, Value>, Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let state = absolute(&ctx.state)?;
    let allow = providers::Allowlist::load(&ctx.providers)?;
    let shown = unit_state(ctx, &root, &state, &allow, path)?.unwrap_or_default();
    let v: Value = serde_json::from_str(&shown).unwrap_or(Value::Null);
    Ok(v["values"]["outputs"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, x)| (k.clone(), x["value"].clone()))
                .collect()
        })
        .unwrap_or_default())
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
/// runs the live checks; `checks` of 0 skips them, and `data` is as for
/// [`sites::build_with`].
///
/// # Errors
///
/// [`Error::Pending`] while `infra/cloudflare`'s state lacks a site's token,
/// or any [`Error`] from building or syncing; never a credential.
pub fn publish(
    ctx: &Context,
    wanted: &[String],
    checks: u32,
    data: &BTreeMap<String, Value>,
) -> Result<Vec<Published>, Error> {
    let root = ctx.root.canonicalize().map_err(io(&ctx.root))?;
    let state = absolute(&ctx.state)?;
    let allow = providers::Allowlist::load(&ctx.providers)?;
    let platform = ctx
        .platform_root()
        .canonicalize()
        .map_err(io(ctx.platform_root()))?;
    let found = sites::find_in(&root, &platform, wanted)?;
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let built = sites::build_in(&root, &platform, &found, data)?;
    let run = match ctx.cloudflare() {
        Some(api) => Some(run_credentials(&api)?),
        None => None,
    };
    let shown = if run.is_some() {
        "{}".to_string()
    } else {
        infra_state(ctx, &root, &state, &allow)?.unwrap_or_else(|| "{}".into())
    };
    let mut out = Vec::new();
    for b in built {
        let found = run
            .clone()
            .or_else(|| sites::credentials(&shown, &b.site, ctx.today));
        let Some((creds, account)) = found else {
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

/// R2's S3 credentials from the run's own token: its ID as the access key, and the SHA-256 of
/// its value as the secret; and the account it sees.
fn run_credentials(api: &cloudflare::Api) -> Result<(s3::Credentials, String), Error> {
    let fail = |message: String| Error::Tool {
        command: "cloudflare".into(),
        message,
    };
    let account = api.account().map_err(fail)?;
    let id = api.token_id(&account).map_err(fail)?;
    Ok((
        s3::Credentials {
            access_key: id,
            secret_key: s3::sha256_hex(api.token().as_bytes()),
        },
        account,
    ))
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

/// Plans each unit as for an apply, and stores the plans under the state's `plans/<id>/` for
/// [`apply_stored`]; see the phase 3 amendment §5 of
/// `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.
///
/// # Errors
///
/// Any [`Error`]; a unit still waiting for a handoff is [`Error::Pending`].
pub fn plan_stored(
    ctx: &Context,
    units: &[Unit],
    now: u64,
) -> Result<(plans::Stored, Vec<UnitPlan>), Error> {
    if units.is_empty() {
        return Err(Error::Plan(
            "no units to plan; name a local unit to plan it".into(),
        ));
    }
    let mut planned = Vec::new();
    let mut files = Vec::new();
    for u in units {
        let (work, _, p) = prepare(ctx, u, true)?;
        let digest = plans::digest(u, &work)?;
        files.push((u, tofu::plan_file(&work), p.changes.clone(), digest));
        planned.push(p);
    }
    let state = absolute(&ctx.state)?;
    plans::prune_completed(&state, now)?;
    let stored = plans::store(&state, &files, now)?;
    Ok((stored, planned))
}

/// Applies the plans stored as `id`, exactly as planned, then removes them; see
/// [`apply_stored_with`].
///
/// # Errors
///
/// As [`apply_stored_with`].
pub fn apply_stored(ctx: &Context, id: &str, now: u64) -> Result<Vec<UnitPlan>, Error> {
    apply_stored_with(ctx, id, now, &mut || Ok(()))
}

/// One attribute a planned resource change sets, as `show` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrChange {
    /// The resource's address.
    pub address: String,
    /// What happens to it: `update`, `create`, `delete`, `replace` or `read`.
    pub action: String,
    /// The attribute, top-level.
    pub attribute: String,
    /// Its value before, or `(sensitive)`, or `null`.
    pub before: String,
    /// Its value after, `(known after apply)`, or `(sensitive)`.
    pub after: String,
    /// Whether changing it forces a replacement.
    pub replaces: bool,
}

/// Whether a `*_sensitive` or `after_unknown` mask marks anything at or under it.
fn masked(mask: &Value) -> bool {
    match mask {
        Value::Bool(b) => *b,
        Value::Array(a) => a.iter().any(masked),
        Value::Object(o) => o.values().any(masked),
        _ => false,
    }
}

/// Each attribute a plan's resource changes set, from `tofu show -json` of the plan; sensitive
/// values are never shown, at any depth.
#[must_use]
pub fn plan_diff(shown: &str) -> Vec<AttrChange> {
    let v: Value = serde_json::from_str(shown).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for rc in v["resource_changes"].as_array().into_iter().flatten() {
        let c = &rc["change"];
        let actions: Vec<&str> = c["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let action = match actions.as_slice() {
            ["no-op"] | [] => continue,
            ["update"] => "update",
            ["create"] => "create",
            ["delete"] => "delete",
            ["read"] => "read",
            _ => "replace",
        };
        let empty = Map::new();
        let before = c["before"].as_object().unwrap_or(&empty);
        let after = c["after"].as_object().unwrap_or(&empty);
        let replace_paths: Vec<String> = c["replace_paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.get(0).and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let b = before.get(k).unwrap_or(&Value::Null);
            let a = after.get(k).unwrap_or(&Value::Null);
            let unknown = masked(&c["after_unknown"][k.as_str()]);
            if b == a && !unknown {
                continue;
            }
            let show = |val: &Value, sensitive: &Value| {
                if masked(sensitive) {
                    "(sensitive)".to_string()
                } else {
                    val.to_string()
                }
            };
            out.push(AttrChange {
                address: rc["address"].as_str().unwrap_or_default().to_string(),
                action: action.to_string(),
                attribute: k.clone(),
                before: show(b, &c["before_sensitive"][k.as_str()]),
                after: if unknown {
                    "(known after apply)".to_string()
                } else {
                    show(a, &c["after_sensitive"][k.as_str()])
                },
                replaces: replace_paths.iter().any(|p| p == k),
            });
        }
    }
    out
}

/// The attribute-level changes of the plans stored as `id`, by unit, without applying them.
///
/// # Errors
///
/// [`Error::Plan`] for an unknown or expired plan; any [`Error`] from `tofu`.
pub fn show_stored(
    ctx: &Context,
    id: &str,
    now: u64,
) -> Result<Vec<(String, Vec<AttrChange>)>, Error> {
    let state = absolute(&ctx.state)?;
    let stored = plans::load(&state, id, now)?;
    let names: Vec<String> = stored.units.iter().map(|s| s.unit.clone()).collect();
    let units = select_in(&ctx.root, ctx.platform_root(), &ctx.providers, &names)?;
    let mut out = Vec::new();
    for (u, s) in units.iter().zip(&stored.units) {
        let (work, env, _) = ready(ctx, u, true)?;
        plans::restore(&state, id, &s.unit, &tofu::plan_file(&work))?;
        let shown = tofu::show_plan(&u.path, &work, &env)?;
        let mut diff = plan_diff(&shown);
        for d in &mut diff {
            for h in env.hidden.iter().filter(|h| h.len() >= 4) {
                d.before = d.before.replace(h.as_str(), "(hidden)");
                d.after = d.after.replace(h.as_str(), "(hidden)");
            }
        }
        out.push((s.unit.clone(), diff));
    }
    Ok(out)
}

/// Applies the plans stored as `id`, exactly as planned, then removes them.
///
/// `checkpoint` runs after each unit is applied and marked, and after the completion record,
/// before the plan is removed. A completed plan returns its recorded result (control-plane ADR §7).
///
/// # Errors
///
/// [`Error::Plan`] for an unknown or expired plan, or a unit whose desired state changed;
/// any other [`Error`] from `tofu`, which also refuses a plan whose state moved on; and any
/// error `checkpoint` returns.
pub fn apply_stored_with(
    ctx: &Context,
    id: &str,
    now: u64,
    checkpoint: &mut dyn FnMut() -> Result<(), Error>,
) -> Result<Vec<UnitPlan>, Error> {
    let state = absolute(&ctx.state)?;
    if let Some(done) = plans::completed(&state, id, now) {
        plans::remove(&state, id)?;
        return Ok(done
            .units
            .into_iter()
            .map(|u| UnitPlan {
                unit: u.unit,
                changes: u.changes,
                note: Some(match u.note {
                    Some(n) => format!("{n} Applied from this plan earlier."),
                    None => "Applied from this plan earlier.".into(),
                }),
            })
            .collect());
    }
    let stored = plans::load(&state, id, now)?;
    let done = plans::applied(&state, id);
    let pending: Vec<&plans::StoredUnit> = stored
        .units
        .iter()
        .filter(|s| !done.contains(&s.unit))
        .collect();
    let names: Vec<String> = pending.iter().map(|s| s.unit.clone()).collect();
    let units = if names.is_empty() {
        Vec::new()
    } else {
        select_in(&ctx.root, ctx.platform_root(), &ctx.providers, &names)?
    };
    let mut ready_units = Vec::new();
    for (u, s) in units.iter().zip(&pending) {
        let (work, env, note) = ready(ctx, u, true)?;
        if plans::digest(u, &work)? != s.digest {
            return Err(Error::Plan(format!(
                "{}'s desired state changed since plan {id}; plan again",
                u.path
            )));
        }
        ready_units.push((u, s, work, env, note));
    }
    let mut applied: Vec<UnitPlan> = stored
        .units
        .iter()
        .filter(|s| done.contains(&s.unit))
        .map(|s| UnitPlan {
            unit: s.unit.clone(),
            changes: s.changes.clone(),
            note: Some("Already applied from this plan.".into()),
        })
        .collect();
    for (u, s, work, env, note) in ready_units {
        plans::restore(&state, id, &s.unit, &tofu::plan_file(&work))?;
        tofu::apply(&u.path, &work, &env)?;
        plans::mark_applied(&state, id, &s.unit)?;
        finish_move(ctx, u)?;
        checkpoint()?;
        applied.push(UnitPlan {
            unit: u.path.clone(),
            changes: s.changes.clone(),
            note,
        });
    }
    plans::complete(
        &state,
        &plans::Completed {
            id: id.to_string(),
            completed: now,
            units: applied
                .iter()
                .map(|u| plans::CompletedUnit {
                    unit: u.unit.clone(),
                    changes: u.changes.clone(),
                    note: u.note.clone(),
                })
                .collect(),
        },
    )?;
    checkpoint()?;
    plans::remove(&state, id)?;
    Ok(applied)
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

/// A Markdown summary of `plans`: addresses, actions and changed attribute names only.
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
                let _ = write!(out, "- {} `{}`", c.action.word(), c.address);
                if c.attributes.is_empty() {
                    out.push('\n');
                } else {
                    let _ = writeln!(out, " ({})", c.attributes.join(", "));
                }
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
