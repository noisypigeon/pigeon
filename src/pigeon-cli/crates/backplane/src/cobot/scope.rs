//! The checks a change needs, read from the workspace on disk and run; the selection is
//! [`crate::core::deploy::scope`]; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::cobot::Error;
use crate::cobot::release::Config;

pub use crate::core::deploy::scope::*;

fn read_toml(path: &Path) -> Result<toml::Value, Error> {
    let io = |message: String| Error::Io {
        path: path.to_path_buf(),
        message,
    };
    fs::read_to_string(path)
        .map_err(|e| io(e.to_string()))?
        .parse()
        .map_err(|e: toml::de::Error| io(e.to_string()))
}

fn crates(root: &Path) -> Result<Crates, Error> {
    let ws = root.join(WORKSPACE);
    let workspace = read_toml(&ws.join("Cargo.toml"))?;
    let members: BTreeMap<String, String> = workspace["workspace"]["dependencies"]
        .as_table()
        .into_iter()
        .flatten()
        .filter_map(|(key, v)| {
            let path = v.get("path")?.as_str()?;
            let package = v
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            Some((key.clone(), format!("{path}/{package}")))
        })
        .collect();
    let mut by_dir = BTreeMap::new();
    let mut deps = BTreeMap::new();
    let mut features = BTreeMap::new();
    for entry in fs::read_dir(ws.join("crates"))
        .map_err(|e| Error::Io {
            path: ws.join("crates"),
            message: e.to_string(),
        })?
        .flatten()
    {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let m = read_toml(&manifest)?;
        let Some(name) = m["package"]["name"].as_str() else {
            continue;
        };
        let dir = entry.file_name().to_string_lossy().into_owned();
        let mine: BTreeSet<String> = ["dependencies", "dev-dependencies"]
            .iter()
            .filter_map(|t| m.get(*t).and_then(toml::Value::as_table))
            .flat_map(|t| t.keys())
            .filter_map(|k| members.get(k))
            .filter_map(|p| p.rsplit('/').next().map(str::to_string))
            .collect();
        if name == "pigeon" {
            features = pigeon_features(&m, &members);
        }
        by_dir.insert(format!("{WORKSPACE}/crates/{dir}/"), name.to_string());
        deps.insert(name.to_string(), mine);
    }
    Ok(Crates {
        by_dir,
        deps,
        features,
    })
}

/// `pigeon`'s features but `default`, each with the workspace packages its `dep:` entries enable.
fn pigeon_features(
    manifest: &toml::Value,
    members: &BTreeMap<String, String>,
) -> BTreeMap<String, BTreeSet<String>> {
    manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .filter(|(f, _)| f.as_str() != "default")
        .map(|(f, list)| {
            let packages = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str()?.strip_prefix("dep:"))
                .filter_map(|k| members.get(k))
                .filter_map(|p| p.rsplit('/').next().map(str::to_string))
                .collect();
            (f.clone(), packages)
        })
        .collect()
}

/// Every check a workspace-wide change runs: the full set the merge gate runs on a head; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §5.
///
/// # Errors
///
/// [`Error::Io`] when the workspace's manifests or the release configuration can't be read.
pub fn full(root: &Path) -> Result<Vec<Job>, Error> {
    let config = Config::load(&root.join(RELEASES))?;
    Ok(full_of(&config, &crates(root)?))
}

/// The checks a change touching `files` needs, in the order to run them; `policy.check`
/// always runs, and a workspace-wide setting runs everything.
///
/// # Errors
///
/// [`Error::Io`] when the workspace's manifests or the release configuration can't be read.
pub fn jobs(root: &Path, files: &[String]) -> Result<Vec<Job>, Error> {
    jobs_with(root, files, None)
}

/// As [`jobs`], with the tests a Rust change runs chosen by `map`, `main`'s test map, when it
/// knows the change; see the 2026-11-11 amendment to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
///
/// # Errors
///
/// As [`jobs`].
pub fn jobs_with(root: &Path, files: &[String], map: Option<&TestMap>) -> Result<Vec<Job>, Error> {
    let config = Config::load(&root.join(RELEASES))?;
    Ok(jobs_of(&config, &crates(root)?, files, map))
}

/// How much of the full set the merge gate of a change touching `files` runs, from the
/// workspace's manifests and `main`'s test map `map`; see [`gate_of`].
///
/// # Errors
///
/// [`Error::Io`] when the workspace's manifests can't be read.
pub fn gate(
    root: &Path,
    files: &[String],
    map: Option<&TestMap>,
) -> Result<crate::core::deploy::gate::Gate, Error> {
    Ok(gate_of(&crates(root)?, files, map))
}

impl TestMap {
    /// The map in `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when it can't be read, [`Error::Tool`] when it isn't a map.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|e| Error::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        serde_json::from_str(&text).map_err(|e| Error::Tool {
            command: format!("test map {}", path.display()),
            message: e.to_string(),
        })
    }
}

fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>, Error> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| Error::Tool {
            command: "git".into(),
            message: e.to_string(),
        })?;
    if !out.status.success() {
        return Err(Error::Tool {
            command: format!("git {}", args.join(" ")),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// The paths a change touches: committed since it left `base`, plus uncommitted and untracked
/// ones, sorted and once each.
///
/// # Errors
///
/// [`Error::Tool`] when git fails, e.g. on an unknown `base`.
pub fn changed(root: &Path, base: &str) -> Result<Vec<String>, Error> {
    let range = format!("{base}...HEAD");
    let mut files = BTreeSet::new();
    for args in [
        &["diff", "--name-only", range.as_str()][..],
        &["diff", "--name-only", "HEAD"],
        &["ls-files", "--others", "--exclude-standard"],
    ] {
        files.extend(git_lines(root, args)?);
    }
    Ok(files.into_iter().collect())
}

/// Runs `jobs` one after another in [`WORKSPACE`], each with its output shown; every job runs,
/// and the names of those that failed are returned.
///
/// # Errors
///
/// [`Error::Tool`] when a job can't start.
pub fn run(root: &Path, jobs: &[Job]) -> Result<Vec<String>, Error> {
    let mut failed = Vec::new();
    for j in jobs {
        let Some((program, args)) = j.argv.split_first() else {
            continue;
        };
        eprintln!("cobot: scope: {}", j.name);
        let status = std::process::Command::new(program)
            .args(args)
            .current_dir(root.join(WORKSPACE))
            .status()
            .map_err(|e| Error::Tool {
                command: program.clone(),
                message: e.to_string(),
            })?;
        if !status.success() {
            failed.push(j.name.clone());
        }
    }
    Ok(failed)
}

/// What an `architecture` site draws from, besides its own unit: everything
/// `architecture.describe` reads.
const DESCRIBED: [&str; 5] = [
    "config.yaml",
    "infra/",
    "catalog/",
    "workloads/",
    ".config/",
];

fn relative(root: &Path, p: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    p.strip_prefix(&root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The instance's manifest sites a change to `main` touching `files` must publish, by unit; `files` of `None`
/// means every site, as on a first deploy. A site's inputs are its unit (the instance's
/// `config.yaml` for a manifest unit, else its directory), its content, and, on the
/// `architecture` template, everything that template draws.
///
/// # Errors
///
/// As [`crate::cobot::sites::find_in`].
pub fn deploys(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    // `workloads/` is the legacy layout, published from its own instance's account.
    let sites: Vec<_> = crate::cobot::sites::find_in(root, root, &[])?
        .into_iter()
        .filter(|s| !s.unit.starts_with("workloads/"))
        .collect();
    let Some(files) = files else {
        return Ok(sites.into_iter().map(|s| s.unit).collect());
    };
    Ok(sites
        .into_iter()
        .filter(|s| {
            let dir = relative(root, &s.dir);
            let unit = if dir.is_empty() {
                "config.yaml".to_string()
            } else {
                format!("{dir}/")
            };
            let content = format!("{}/", relative(root, &s.content));
            let mut inputs = vec![unit, content];
            if s.template == "architecture" {
                inputs.extend(DESCRIBED.iter().map(|p| (*p).to_string()));
            }
            files.iter().any(|f| {
                inputs.iter().any(|i| {
                    i != "/" && (f == i || (i.ends_with('/') && f.starts_with(i.as_str())))
                })
            })
        })
        .map(|s| s.unit)
        .collect())
}

/// The Workers a change to `main` touching `files` must deploy, by name; `files` of `None`
/// means every Worker. A Worker's inputs are its spec's directory, its migrations, its
/// Dockerfile, the workspace's build settings, and the crates its entry crate builds from.
///
/// # Errors
///
/// As [`crate::cobot::workers::discover`], or a workspace manifest that can't be read.
pub fn workers(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    let found = crate::cobot::workers::discover(root)?;
    let Some(files) = files else {
        return Ok(found
            .iter()
            .map(|f| crate::cobot::workers::deployed_name(root, f))
            .collect());
    };
    let crates = crates(root)?;
    let mut out = Vec::new();
    for f in found {
        let mut inputs: Vec<String> = vec![format!("{}/", f.dir.trim_end_matches('/'))];
        inputs.extend(
            f.spec
                .bindings
                .d1
                .values()
                .filter_map(|d| d.migrations.as_ref())
                .map(|m| format!("{}/", m.trim_end_matches('/'))),
        );
        inputs.extend(f.spec.containers.iter().map(|c| c.image.dockerfile.clone()));
        inputs.extend(EVERYTHING.iter().map(|p| (*p).to_string()));
        let entry = format!("{}/", f.spec.entry.krate.trim_end_matches('/'));
        if let Some(package) = crates.by_dir.get(&entry) {
            let mut wanted = BTreeSet::from([package.clone()]);
            loop {
                let more: Vec<String> = wanted
                    .iter()
                    .flat_map(|p| crates.deps.get(p).into_iter().flatten())
                    .filter(|d| !wanted.contains(*d))
                    .cloned()
                    .collect();
                if more.is_empty() {
                    break;
                }
                wanted.extend(more);
            }
            inputs.extend(
                crates
                    .by_dir
                    .iter()
                    .filter(|(_, p)| wanted.contains(*p))
                    .map(|(d, _)| d.clone()),
            );
        } else {
            inputs.push(entry);
        }
        let hit = files.iter().any(|file| {
            inputs
                .iter()
                .any(|i| file == i || (i.ends_with('/') && file.starts_with(i.as_str())))
        });
        if hit {
            out.push(crate::cobot::workers::deployed_name(root, &f));
        }
    }
    Ok(out)
}
