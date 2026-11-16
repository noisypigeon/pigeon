//! The checks a change needs, read from the workspace on disk and run; the selection is
//! [`crate::core::deploy::scope`]; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::infra::Error;
use crate::release::Config;
use crate::release::LoadConfig;

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

/// The Rust workspace's directory, from `root`: the root itself when its `Cargo.toml` has
/// `[workspace]`, else [`WORKSPACE`]; empty for the root.
#[must_use]
pub fn workspace_dir(root: &Path) -> String {
    let at_root = fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Value>().ok())
        .is_some_and(|m| m.get("workspace").is_some());
    if at_root {
        String::new()
    } else {
        WORKSPACE.into()
    }
}

/// The Rust workspace's directory under `root`, where every check runs.
#[must_use]
pub fn workspace(root: &Path) -> std::path::PathBuf {
    root.join(workspace_dir(root))
}

/// The crate directories, from the workspace, that `members` names, without `exclude`'s: an
/// entry ending `/*` names each directory under it holding a `Cargo.toml`.
fn member_dirs(ws: &Path, members: &[&str], exclude: &[&str]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for m in members {
        let m = m.trim_end_matches('/');
        if let Some(parent) = m.strip_suffix("/*") {
            for entry in fs::read_dir(ws.join(parent))
                .into_iter()
                .flatten()
                .flatten()
            {
                out.insert(format!("{parent}/{}", entry.file_name().to_string_lossy()));
            }
        } else {
            out.insert(m.to_string());
        }
    }
    out.into_iter()
        .filter(|d| !exclude.iter().any(|x| x.trim_end_matches('/') == d))
        .filter(|d| ws.join(d).join("Cargo.toml").is_file())
        .collect()
}

fn strs(v: Option<&toml::Value>) -> Vec<&str> {
    v.and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect()
}

fn crates(root: &Path) -> Result<Crates, Error> {
    let dir = workspace_dir(root);
    let ws = root.join(&dir);
    let workspace = read_toml(&ws.join("Cargo.toml"))?;
    let mut layout: Layout = workspace
        .get("workspace")
        .and_then(|w| w.get("metadata"))
        .and_then(|m| m.get("pigeon"))
        .cloned()
        .map(toml::Value::try_into)
        .transpose()
        .map_err(|e: toml::de::Error| Error::Io {
            path: ws.join("Cargo.toml"),
            message: format!("[workspace.metadata.pigeon]: {e}"),
        })?
        .unwrap_or_default();
    layout.workspace.clone_from(&dir);
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
    let mut tests = BTreeMap::new();
    let table = workspace.get("workspace");
    let found = member_dirs(
        &ws,
        &strs(table.and_then(|w| w.get("members"))),
        &strs(table.and_then(|w| w.get("exclude"))),
    );
    for member in found {
        let m = read_toml(&ws.join(&member).join("Cargo.toml"))?;
        let Some(name) = m
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        let mine: BTreeSet<String> = ["dependencies", "dev-dependencies"]
            .iter()
            .filter_map(|t| m.get(*t).and_then(toml::Value::as_table))
            .flat_map(|t| t.keys())
            .filter_map(|k| members.get(k))
            .filter_map(|p| p.rsplit('/').next().map(str::to_string))
            .collect();
        if name == layout.cli {
            features = pigeon_features(&m, &members);
        }
        by_dir.insert(layout.in_workspace(&format!("{member}/")), name.to_string());
        tests.insert(name.to_string(), test_targets(&ws.join(&member), &m));
        deps.insert(name.to_string(), mine);
    }
    Ok(Crates {
        by_dir,
        deps,
        features,
        tests,
        layout,
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

/// Reads a [`TestMap`] from disk.
pub trait ReadTestMap: Sized {
    /// The map in `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when it can't be read, [`Error::Tool`] when it isn't a map.
    fn read(path: &Path) -> Result<Self, Error>;
}

impl ReadTestMap for TestMap {
    fn read(path: &Path) -> Result<Self, Error> {
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

/// Runs `jobs` one after another in the workspace, each with its output shown; every job runs,
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
            .current_dir(workspace(root))
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

fn relative(root: &Path, p: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    p.strip_prefix(&root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The instance's manifest sites a change to `main` touching `files` must publish, by unit; `files` of `None`
/// means every site, as on a first deploy. A site's inputs are its unit (the instance's
/// `config.yaml` for a manifest unit, else its directory), its content, and its template's
/// directory when it pins `tree`; a site on the `architecture` template publishes on every
/// deploy.
///
/// # Errors
///
/// As [`crate::sites::find_in`].
pub fn deploys(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    // `workloads/` is the legacy layout, published from its own instance's account.
    let sites: Vec<_> = crate::sites::find_in(root, root, &[])?
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
            if s.version == crate::sites::TREE {
                inputs.push(format!("{}/", s.project()));
            }
            // Its footer names the deployed SHA, so it publishes on every deploy; see
            // .claude/decisions/2026-11-15-runtime-architecture-coverage-agents-deploys.md §3.
            if s.template == "architecture" {
                return true;
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
/// As [`crate::workers::discover`], or a workspace manifest that can't be read.
pub fn workers(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    let found = crate::workers::discover(root)?;
    let Some(files) = files else {
        return Ok(found
            .iter()
            .map(|f| crate::workers::deployed_name(root, f))
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
        inputs.extend(crates.layout.everything());
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
            out.push(crate::workers::deployed_name(root, &f));
        }
    }
    Ok(out)
}

/// The integration test targets of the crate at `dir` with manifest `m`: its `[[test]]` names,
/// and, unless `autotests = false`, each `tests/<x>.rs` and `tests/<x>/main.rs`.
fn test_targets(dir: &Path, m: &toml::Value) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = m
        .get("test")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("name")?.as_str().map(str::to_string))
        .collect();
    if m["package"].get("autotests").and_then(toml::Value::as_bool) != Some(false) {
        for e in fs::read_dir(dir.join("tests"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = e.path();
            let stem = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() && p.join("main.rs").is_file() {
                out.insert(stem);
            } else if let Some(s) = stem.strip_suffix(".rs") {
                out.insert(s.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{crates, workspace_dir};

    fn write(root: &std::path::Path, path: &str, text: &str) {
        let p = root.join(path);
        fs::create_dir_all(p.parent().expect("parent")).expect("dirs");
        fs::write(p, text).expect("write");
    }

    #[test]
    fn a_workspace_at_the_root_is_read_from_its_members_and_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        assert_eq!(workspace_dir(root), "src/pigeon-cli");
        write(
            root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"src/*\"]\nexclude = [\"src/pigeon-ios\"]\n\n\
             [workspace.dependencies]\nagent = { path = \"src/pigeon-agent\", package = \"pigeon-agent\" }\n\n\
             [workspace.metadata.pigeon]\ncli = \"pigeon-harness\"\nrun = [\"run\"]\nlater = 1\n",
        );
        write(
            root,
            "src/pigeon-agent/Cargo.toml",
            "[package]\nname = \"pigeon-agent\"\n",
        );
        write(
            root,
            "src/pigeon-harness/Cargo.toml",
            "[package]\nname = \"pigeon-harness\"\n\n[dependencies]\nagent = { workspace = true, optional = true }\n\n\
             [features]\ndefault = [\"infra\"]\ninfra = [\"dep:agent\"]\n",
        );
        write(
            root,
            "src/pigeon-ios/Cargo.toml",
            "[package]\nname = \"pigeon-ios\"\n",
        );
        write(root, "src/notes/README.md", "");
        assert_eq!(workspace_dir(root), "");
        let c = crates(root).expect("crates");
        assert_eq!(
            c.by_dir.keys().collect::<Vec<_>>(),
            ["src/pigeon-agent/", "src/pigeon-harness/"]
        );
        assert_eq!(
            c.deps["pigeon-harness"],
            ["pigeon-agent".to_string()].into()
        );
        assert_eq!(c.features["infra"], ["pigeon-agent".to_string()].into());
        assert_eq!(c.layout.cli, "pigeon-harness");
        assert_eq!(c.layout.run, ["run"]);
        assert_eq!(c.layout.root(), ".");
    }
}
