//! The judge, `pigeon-sentinel`, run as its own binary; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1-§2.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The variable naming the binary: set by `pigeon-start` in job Containers, or by the caller.
pub const VAR: &str = "PIGEON_SENTINEL";

/// The binary's name.
pub const BINARY: &str = crate::cobot::ship_wire::SENTINEL_BINARY;

/// Where `pigeon-sentinel` is: `var`'s [`VAR`], else beside the running executable or in its
/// parent directory, else its bare name, found on `PATH`.
#[must_use]
pub fn locate(var: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(p) = var(VAR).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        let here = exe.parent();
        for dir in here.into_iter().chain(here.and_then(Path::parent)) {
            let candidate = dir.join(BINARY);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(BINARY)
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Policies that ran.
    pub ran: u64,
    /// Pull-request policies skipped for want of a pull request.
    pub skipped: u64,
    /// Every violation, in order.
    pub violations: Vec<String>,
}

/// A policy and what it targets, as `describe` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    /// Its name, the file's stem.
    pub name: String,
    /// `pull-request`, `change`, `tree`, `file`, `files` or `dir`.
    pub kind: String,
    /// The target's paths.
    pub paths: Vec<String>,
}

/// Why the judge gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// It isn't there, or didn't start.
    Missing(String),
    /// It ran but refused: an invalid policy, or a report that isn't one.
    Refused(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(m) | Self::Refused(m) => f.write_str(m),
        }
    }
}

fn run(binary: &Path, args: &[String], env: &[(&str, String)]) -> Result<Value, Failure> {
    let shown = binary.display();
    let ran = crate::cobot::ci::run_program(binary, args, env).map_err(Failure::Missing)?;
    if !matches!(ran.code, Some(0 | 1)) {
        return Err(Failure::Refused(format!(
            "{shown} couldn't judge: {}",
            ran.stderr.trim()
        )));
    }
    serde_json::from_str(&ran.stdout)
        .map_err(|e| Failure::Refused(format!("{shown}'s answer isn't JSON: {e}")))
}

/// Checks the tree at `root` against `policies`, with `pull_request`'s fields (`PR_TITLE` and
/// the rest) when given.
///
/// # Errors
///
/// When the binary is missing or refuses.
pub fn check(
    binary: &Path,
    root: &Path,
    policies: &Path,
    pull_request: Option<&[(&str, String)]>,
) -> Result<Report, Failure> {
    let mut args: Vec<String> = vec!["check".into(), "--json".into()];
    if pull_request.is_some() {
        args.push("--pull-request".into());
    }
    args.extend([
        "--root".into(),
        root.display().to_string(),
        "--policies".into(),
        policies.display().to_string(),
    ]);
    let v = run(binary, &args, pull_request.unwrap_or_default())?;
    let count = |k: &str| v[k].as_u64().unwrap_or_default();
    Ok(Report {
        ran: count("ran"),
        skipped: count("skipped"),
        violations: v["violations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    })
}

/// Each policy in `policies` and what it targets.
///
/// # Errors
///
/// When the binary is missing or refuses.
pub fn describe(binary: &Path, policies: &Path) -> Result<Vec<Described>, Failure> {
    let args = [
        "describe".to_string(),
        "--policies".into(),
        policies.display().to_string(),
    ];
    let v = run(binary, &args, &[])?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|p| Described {
            name: p["name"].as_str().unwrap_or_default().to_string(),
            kind: p["target"]["kind"].as_str().unwrap_or_default().to_string(),
            paths: p["target"]["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
        })
        .collect())
}

/// The binary's version, as `pigeon-sentinel --version` prints it; none when it doesn't run.
#[must_use]
pub fn version(binary: &Path) -> Option<String> {
    let ran = crate::cobot::ci::run_program(binary, &["--version".to_string()], &[]).ok()?;
    (ran.code == Some(0)).then(|| {
        ran.stdout
            .trim()
            .trim_start_matches(BINARY)
            .trim()
            .to_string()
    })
}
