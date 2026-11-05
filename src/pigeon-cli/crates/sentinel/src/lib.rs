//! A policy engine that checks a repository against YAML policies.
//!
//! See `.claude/decisions/2026-11-02-development-cargo-workspace-mise-toolchain.md`.

pub mod definition;
mod markdown;
pub mod policy;
pub mod report;
mod rules;
mod target;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub use definition::{Definition, Invocation};
pub use policy::Policy;
pub use report::{Report, Violation};

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A policy file that failed to load, or a file a policy could not read.
#[derive(Debug)]
pub enum Error {
    /// Reading a file or directory failed.
    Io {
        /// The path being read.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A policy file is not a valid policy.
    Policy {
        /// The policy file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Policy { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

/// A parsed policy and the name it reports under, its file stem.
#[derive(Debug)]
pub struct LoadedPolicy {
    /// The policy file's stem.
    pub name: String,
    /// The policy.
    pub policy: Policy,
}

/// A pull request's fields; `None` where the environment did not supply one.
#[derive(Debug, Default, Clone)]
pub struct PullRequest {
    /// The title.
    pub title: Option<String>,
    /// The body.
    pub body: Option<String>,
    /// The number.
    pub number: Option<String>,
    /// The changed paths, one per line.
    pub files: Option<String>,
    /// A change's branch, `change/<n>-<slug>`.
    pub branch: Option<String>,
    /// A change's approved scope: path globs, one per line.
    pub scope: Option<String>,
}

/// Loads and validates one policy file.
///
/// # Errors
///
/// [`Error::Io`] if the file cannot be read, [`Error::Policy`] if it is invalid.
pub fn load_file(path: &Path) -> Result<LoadedPolicy, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let policy = Policy::parse(&text).map_err(|message| Error::Policy {
        path: path.to_path_buf(),
        message,
    })?;
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(LoadedPolicy { name, policy })
}

/// Loads every `*.yaml` policy in `dir`, sorted by name.
///
/// # Errors
///
/// [`Error::Io`] if `dir` cannot be read, [`Error::Policy`] if any policy is
/// invalid or there are none.
pub fn load_dir(dir: &Path) -> Result<Vec<LoadedPolicy>, Error> {
    let io = |source| Error::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.extension().is_some_and(|e| e == "yaml") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(Error::Policy {
            path: dir.to_path_buf(),
            message: "no *.yaml policies found".into(),
        });
    }
    paths.iter().map(|p| load_file(p)).collect()
}

/// Every `*.yaml` definition in `dir`, by name (its file stem), per the control-plane ADR's
/// "A run's instance root, built from D1" amendment §2.
///
/// # Errors
///
/// [`Error::Io`] if `dir` or a file cannot be read, [`Error::Policy`] if a definition is
/// invalid.
pub fn load_definitions(dir: &Path) -> Result<BTreeMap<String, Definition>, Error> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    let mut definitions = BTreeMap::new();
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let path = entry.map_err(io(dir))?.path();
        if !(path.extension().is_some_and(|e| e == "yaml") && path.is_file()) {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        let definition = Definition::parse(&text).map_err(|message| Error::Policy {
            path: path.clone(),
            message,
        })?;
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        definitions.insert(name, definition);
    }
    Ok(definitions)
}

/// The policies `invocations` ask of `definitions`, each reporting under its definition's name.
///
/// # Errors
///
/// [`Error::Policy`], at `source`, for an unknown name or an invocation its definition refuses.
pub fn invoke(
    definitions: &BTreeMap<String, Definition>,
    invocations: &[Invocation],
    source: &Path,
) -> Result<Vec<LoadedPolicy>, Error> {
    invocations
        .iter()
        .map(|invocation| {
            let refused = |message: String| Error::Policy {
                path: source.to_path_buf(),
                message: format!("policy `{}`: {message}", invocation.name),
            };
            let definition = definitions
                .get(&invocation.name)
                .ok_or_else(|| refused("no such definition".into()))?;
            Ok(LoadedPolicy {
                name: invocation.name.clone(),
                policy: definition.invoke(invocation).map_err(refused)?,
            })
        })
        .collect()
}

/// Runs `policies` against `root`. Pull-request policies run only when
/// `pull_request` is given.
///
/// # Errors
///
/// [`Error::Io`] if a targeted directory exists but cannot be listed.
pub fn check(
    policies: &[LoadedPolicy],
    root: &Path,
    pull_request: Option<&PullRequest>,
) -> Result<Report, Error> {
    let mut report = Report::default();
    for loaded in policies {
        if target::run(loaded, root, pull_request, &mut report)? {
            report.ran += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}
