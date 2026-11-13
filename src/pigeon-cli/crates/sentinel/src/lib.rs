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

/// An eviction candidate: each field the control plane read from the change's record and its
/// session's ticket, as text; see
/// `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4.
pub type Candidate = std::collections::BTreeMap<String, String>;

/// One eviction policy's verdict on a candidate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Verdict {
    /// The policy.
    pub policy: String,
    /// Why it doesn't grant; none means it does.
    pub violations: Vec<String>,
}

/// Each `eviction` policy's verdict on `candidate`, in file-name order, and the first that
/// grants it, which is the grant: no violations.
#[must_use]
pub fn evict(policies: &[LoadedPolicy], candidate: &Candidate) -> (Vec<Verdict>, Option<String>) {
    let mut verdicts: Vec<Verdict> = policies
        .iter()
        .filter(|p| {
            matches!(
                p.policy.target,
                policy::Target::Named(policy::NamedTarget::Eviction)
            )
        })
        .map(|p| {
            let mut out = Vec::new();
            rules::run_eviction(p, candidate, &mut out);
            Verdict {
                policy: p.name.clone(),
                violations: out.iter().map(|v| v.message.clone()).collect(),
            }
        })
        .collect();
    verdicts.sort_by(|a, b| a.policy.cmp(&b.policy));
    let granted = verdicts
        .iter()
        .find(|v| v.violations.is_empty())
        .map(|v| v.policy.clone());
    (verdicts, granted)
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

/// Each policy's name and what it targets, as JSON: `[{name, target: {kind, paths}}]`, `kind`
/// one of `pull-request`, `change`, `tree`, `file`, `files` and `dir`; for readers that don't
/// link this library, such as backplane's `architecture.describe`.
#[must_use]
pub fn describe(policies: &[LoadedPolicy]) -> serde_json::Value {
    use policy::{NamedTarget, Target};
    let shown = |p: &Path| p.display().to_string();
    serde_json::Value::Array(
        policies
            .iter()
            .map(|p| {
                let (kind, paths): (&str, Vec<String>) = match &p.policy.target {
                    Target::Named(NamedTarget::PullRequest) => ("pull-request", Vec::new()),
                    Target::Named(NamedTarget::Change) => ("change", Vec::new()),
                    Target::Named(NamedTarget::Tree) => ("tree", Vec::new()),
                    Target::Named(NamedTarget::Eviction) => ("eviction", Vec::new()),
                    Target::File(f) => ("file", vec![shown(&f.file)]),
                    Target::Files(f) => ("files", f.files.iter().map(|p| shown(p)).collect()),
                    Target::Dir(d) => ("dir", vec![shown(&d.dir)]),
                };
                serde_json::json!({
                    "name": p.name,
                    "target": { "kind": kind, "paths": paths },
                })
            })
            .collect(),
    )
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
