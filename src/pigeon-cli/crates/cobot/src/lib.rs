//! Infrastructure as code; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`.

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
    /// The passphrase is missing or too short.
    Passphrase,
    /// A file could not be read or written.
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
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Tofu { unit, message } => write!(f, "{unit}: {message}"),
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

/// Validates every unit.
///
/// # Errors
///
/// [`Error::Invalid`] listing every problem.
pub fn validate(root: &Path) -> Result<Vec<Unit>, Error> {
    let problems = unit::validate(root);
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
pub fn select(root: &Path, wanted: &[String]) -> Result<Vec<Unit>, Error> {
    let units = validate(root)?;
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

/// The rendered `main.tf.json` for `unit`, with state under `state`.
#[must_use]
pub fn render_json(unit: &Unit, state: &Path) -> Value {
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
            "backend": { "local": { "path": state.join(&unit.path).join("terraform.tfstate").display().to_string() } }
        },
        "module": modules
    })
}

/// Writes `<root>/.cobot/<unit>/main.tf.json`; returns the work directory.
///
/// # Errors
///
/// [`Error::Io`] if the work directory cannot be written.
pub fn render(root: &Path, unit: &Unit, state: &Path) -> Result<PathBuf, Error> {
    let work = root.join(WORK_DIR).join(&unit.path);
    fs::create_dir_all(&work).map_err(io(&work))?;
    let file = work.join("main.tf.json");
    let text = serde_json::to_string_pretty(&render_json(unit, state)).map_err(|e| Error::Io {
        path: file.clone(),
        message: e.to_string(),
    })?;
    fs::write(&file, text + "\n").map_err(io(&file))?;
    Ok(work)
}

fn absolute(path: &Path) -> Result<PathBuf, Error> {
    fs::create_dir_all(path).map_err(io(path))?;
    path.canonicalize().map_err(io(path))
}

/// Plans each unit against state under `state`.
///
/// # Errors
///
/// Any [`Error`]; `tofu` failures are redacted.
pub fn plan(
    root: &Path,
    units: &[Unit],
    state: &Path,
    passphrase: &str,
) -> Result<Vec<UnitPlan>, Error> {
    let state = absolute(state)?;
    units
        .iter()
        .map(|u| {
            let work = render(root, u, &state)?;
            tofu::plan(&u.path, &work, passphrase)
        })
        .collect()
}

/// Plans, then applies each unit with changes.
///
/// # Errors
///
/// Any [`Error`]; `tofu` failures are redacted.
pub fn apply(
    root: &Path,
    units: &[Unit],
    state: &Path,
    passphrase: &str,
) -> Result<Vec<UnitPlan>, Error> {
    let plans = plan(root, units, state, passphrase)?;
    for (u, p) in units.iter().zip(&plans) {
        if p.has_changes() {
            tofu::apply(&u.path, &root.join(WORK_DIR).join(&u.path), passphrase)?;
        }
    }
    Ok(plans)
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
