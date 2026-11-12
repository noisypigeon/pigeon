//! Release units and generated changelog tiers: git, `cargo tree` and every read and write, around
//! [`crate::core::release`]; see `.claude/decisions/2026-11-02-release-crate-units-and-images.md`.

mod git;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cobot::Error;
use crate::core::release::lines::{
    bumped, first_lines, next_lines, package_manifest, package_name, read_version, released,
    setting_lines, tag_rows, tier_text, tree_of, user_only,
};

pub use crate::core::release::*;

impl Config {
    /// Loads and checks `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if unreadable or invalid.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
        Self::parse(&text).map_err(|message| Error::Io {
            path: path.to_path_buf(),
            message,
        })
    }
}

/// Commits in `range` that changed a workspace-wide build setting.
fn settings_commits<'a>(root: &Path, config: &Config, range: &'a [Commit]) -> Vec<&'a Commit> {
    let mut out = Vec::new();
    for c in range {
        for setting in &config.settings {
            if !c.paths.iter().any(|p| p == &setting.file) {
                continue;
            }
            let after = git::show(root, &c.sha, &setting.file);
            let before = git::show(root, &format!("{}^", c.sha), &setting.file);
            if setting_lines(&after, setting) != setting_lines(&before, setting) {
                out.push(c);
                break;
            }
        }
    }
    out
}

static SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// A unit's resolved dependencies at `rev`, workspace crates by name, and
/// those crates' repository-relative paths.
fn dependency_tree(
    root: &Path,
    rev: &str,
    cargo: &CargoTree,
    package: &str,
    features: &str,
) -> Result<(BTreeSet<String>, Vec<String>), Error> {
    let dir = std::env::temp_dir().join(format!(
        "cobot-tree-{}-{}",
        std::process::id(),
        SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let io = |source: std::io::Error| Error::Io {
        path: dir.clone(),
        message: source.to_string(),
    };
    fs::create_dir_all(&dir).map_err(io)?;
    let result = (|| {
        let archive = dir.join("tree.tar");
        git::run(
            root,
            &[
                "archive",
                "--format=tar",
                "-o",
                &archive.display().to_string(),
                rev,
                "--",
                &cargo.workspace,
            ],
        )?;
        tool(&dir, "tar", &["-xf", "tree.tar"])?;
        let manifest = dir.join(&cargo.workspace).join("Cargo.toml");
        let mut args = vec![
            "tree".to_string(),
            "--locked".into(),
            "--manifest-path".into(),
            manifest.display().to_string(),
            "-p".into(),
            package.to_string(),
            "--no-default-features".into(),
            "-e".into(),
            "normal,build,features".into(),
            "--prefix".into(),
            "none".into(),
        ];
        if !features.is_empty() {
            args.extend(["--features".into(), features.to_string()]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = tool(&dir, "cargo", &args)?;
        let local = format!(" ({}/", dir.display());
        Ok(tree_of(&out, &local, package))
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

/// The path, at `rev`, of the manifest of `cargo`'s package, found by its name.
fn package_path(root: &Path, rev: &str, cargo: &CargoTree) -> Result<Option<String>, Error> {
    let listed = git::run(
        root,
        &["ls-tree", "-r", "--name-only", rev, "--", &cargo.workspace],
    )?;
    let manifests: Vec<(String, String)> = listed
        .lines()
        .filter(|p| p.ends_with("/Cargo.toml"))
        .map(|p| (p.to_string(), git::show(root, rev, p)))
        .collect();
    Ok(package_manifest(&manifests, &cargo.package).map(str::to_string))
}

/// Runs `program` in `dir` and returns stdout.
fn tool(dir: &Path, program: &str, args: &[&str]) -> Result<String, Error> {
    let output = Command::new(program)
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| Error::Tool {
            command: format!("{program} {}", args.join(" ")),
            message: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(Error::Tool {
            command: format!("{program} {}", args.join(" ")),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn user_commits(root: &Path, from: Option<&str>, to: &str) -> Result<Vec<Commit>, Error> {
    Ok(user_only(git::commits(root, from, to)?))
}

/// A unit's release, if it has one, and that release's changelog lines.
fn unit_release(
    root: &Path,
    config: &Config,
    name: &str,
    unit: &Unit,
    to: &str,
    files: &mut BTreeMap<PathBuf, String>,
) -> Result<Option<(Released, Vec<String>)>, Error> {
    let manifest_path = root.join(&unit.manifest);
    let manifest = fs::read_to_string(&manifest_path).map_err(|source| Error::Io {
        path: manifest_path.clone(),
        message: source.to_string(),
    })?;
    let Some((start, end, patch, major_minor)) = read_version(&manifest) else {
        return Err(Error::Io {
            path: unit.manifest.clone(),
            message: "no `version = \"X.Y.Z\"` line".into(),
        });
    };
    let tag = git::last_tag(root, name)?;
    let Some(last) = &tag else {
        let user = user_commits(root, None, to)?;
        let all = git::commits(root, None, to)?;
        let lines = first_lines(&config.repository, unit, &user, all.last());
        let version = format!("{major_minor}.{patch}");
        return Ok(Some((released(name, unit, version), lines)));
    };
    let range = user_commits(root, Some(last), to)?;
    let mut prefixes = unit.paths.clone();
    let mut dependencies_changed = false;
    if let Some(cargo) = &unit.cargo {
        // By the name its manifest has at each revision, so a renamed package still compares.
        let manifest = package_path(root, to, cargo)?;
        let name_at = |rev: &str| {
            manifest
                .as_deref()
                .and_then(|m| package_name(&git::show(root, rev, m)))
                .unwrap_or_else(|| cargo.package.clone())
        };
        let (now, workspace) = dependency_tree(root, to, cargo, &name_at(to), &unit.features)?;
        prefixes.extend(workspace);
        let (before, _) = dependency_tree(root, last, cargo, &name_at(last), &unit.features)?;
        dependencies_changed = before != now;
    }
    let settings: Vec<&str> = settings_commits(root, config, &range)
        .into_iter()
        .map(|c| c.sha.as_str())
        .collect();
    let lines = next_lines(
        &config.repository,
        unit,
        &range,
        &prefixes,
        &settings,
        dependencies_changed,
    );
    if lines.is_empty() {
        return Ok(None);
    }
    let version = format!("{major_minor}.{}", patch + 1);
    files.insert(
        unit.manifest.clone(),
        bumped(&manifest, start, end, &version),
    );
    Ok(Some((released(name, unit, version), lines)))
}

/// Renders every tier, recording the ones that changed in `files`.
fn render_tiers(
    root: &Path,
    config: &Config,
    from: Option<&str>,
    to: &str,
    releases: &[(Released, Vec<String>)],
    files: &mut BTreeMap<PathBuf, String>,
    removed: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let today = git::date(root, to)?;
    let range = user_commits(root, from, to)?;
    let mut history: Option<Vec<Commit>> = None;
    for tier in &config.tiers {
        let at = |p: &Path| root.join(p).is_file();
        let moved = match &tier.from {
            Some(old) if at(old) && at(&tier.file) => {
                return Err(Error::Io {
                    path: root.join(old),
                    message: format!(
                        "both `{}` and `{}` exist; a tier lives in one place",
                        old.display(),
                        tier.file.display()
                    ),
                });
            }
            Some(old) if at(old) => Some(old.clone()),
            _ => None,
        };
        let existing = match &moved {
            Some(old) => Some(fs::read_to_string(root.join(old)).map_err(|e| Error::Io {
                path: root.join(old),
                message: format!("cannot read the tier to move: {e}"),
            })?),
            None => fs::read_to_string(root.join(&tier.file)).ok(),
        };
        let commits: &[Commit] = match (&tier.paths, &existing, &mut history) {
            (None, _, _) => &[],
            (Some(_), Some(_), _) => &range,
            (Some(_), None, Some(all)) => all,
            (Some(_), None, slot @ None) => slot.insert(user_commits(root, None, to)?),
        };
        let rendered = tier_text(
            tier,
            existing.as_deref(),
            commits,
            &config.repository,
            releases,
            &today,
        );
        if let Some(old) = moved {
            files.insert(tier.file.clone(), rendered);
            removed.push(old);
        } else if existing.as_deref() != Some(rendered.as_str()) {
            files.insert(tier.file.clone(), rendered);
        }
    }
    Ok(())
}

/// Every release tag of every unit, each unit's newest version first.
///
/// # Errors
///
/// [`Error::Git`] if git fails.
pub fn tags(root: &Path, config: &Config) -> Result<Vec<TagStatus>, Error> {
    let mut rows = Vec::new();
    for (name, unit) in &config.units {
        rows.extend(tag_rows(config, name, unit, git::tags(root, name)?));
    }
    Ok(rows)
}

/// Runs a release over `from..to`; with `dry_run`, writes nothing. Without
/// `from`, the range starts after the last `chore(release):` commit.
///
/// # Errors
///
/// [`Error::Git`] if git fails, [`Error::Io`] if a file cannot be read or
/// written, or a manifest has no plain `version` line.
pub fn run(
    root: &Path,
    config: &Config,
    from: Option<&str>,
    to: &str,
    notes_dir: Option<&Path>,
    dry_run: bool,
) -> Result<Outcome, Error> {
    let last_release = match from {
        Some(_) => None,
        None => git::last_with_subject(root, to, RELEASE_PREFIX)?,
    };
    let from = from.or(last_release.as_deref());
    let mut files: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut releases = Vec::new();
    for (name, unit) in &config.units {
        if let Some(r) = unit_release(root, config, name, unit, to, &mut files)? {
            releases.push(r);
        }
    }
    let mut removed = Vec::new();
    render_tiers(root, config, from, to, &releases, &mut files, &mut removed)?;
    removed.extend(
        config
            .retired
            .iter()
            .filter(|p| root.join(p).is_file())
            .cloned(),
    );
    if !dry_run {
        for (rel, content) in &files {
            let path = root.join(rel);
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|source| Error::Io {
                    path: dir.to_path_buf(),
                    message: source.to_string(),
                })?;
            }
            fs::write(&path, content).map_err(|source| Error::Io {
                path,
                message: source.to_string(),
            })?;
        }
        for rel in &removed {
            let path = root.join(rel);
            fs::remove_file(&path).map_err(|source| Error::Io {
                path,
                message: source.to_string(),
            })?;
        }
        if let Some(dir) = notes_dir {
            fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.to_path_buf(),
                message: source.to_string(),
            })?;
            for (released, lines) in &releases {
                let path = dir.join(format!("{}.md", released.unit));
                fs::write(&path, lines.join("\n") + "\n").map_err(|source| Error::Io {
                    path,
                    message: source.to_string(),
                })?;
            }
        }
    }
    Ok(Outcome {
        released: releases.into_iter().map(|(r, _)| r).collect(),
        written: files.into_keys().chain(removed).collect(),
    })
}
