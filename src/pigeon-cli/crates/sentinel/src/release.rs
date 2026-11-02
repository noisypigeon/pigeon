//! Release units and generated changelog tiers; see
//! `.claude/decisions/2026-11-02-release-crate-units-and-images.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::changelog::Changelog;
use crate::git::{self, Commit};

const RELEASE_PREFIX: &str = "chore(release):";

/// `.config/releases.yaml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// One line saying what the file configures.
    pub description: String,
    /// The repository URL that changelog links point into.
    pub repository: String,
    /// Release units by name.
    pub units: BTreeMap<String, Unit>,
    /// Changelog tiers.
    pub tiers: Vec<Tier>,
    /// Workspace-wide build settings whose change releases every unit.
    #[serde(default)]
    pub settings: Vec<Setting>,
}

/// Lines of a file that are build settings: whole tables, and single keys.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    /// The file, relative to the root.
    pub file: String,
    /// Tables whose header matches this are settings, header and body.
    #[serde(default)]
    pub tables: Option<String>,
    /// Lines matching this, in any table, are settings.
    #[serde(default)]
    pub keys: Option<String>,
}

/// Where a unit's dependency tree comes from.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CargoTree {
    /// The workspace directory, relative to the root.
    pub workspace: String,
    /// The package whose tree, with the unit's features, is compared.
    pub package: String,
}

/// A separately versioned, separately imaged crate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    /// The crate's `Cargo.toml`, holding its version.
    pub manifest: PathBuf,
    /// The `pigeon` features its image is built with; empty for non-crate units.
    #[serde(default)]
    pub features: String,
    /// The image repository; none publishes no image.
    #[serde(default)]
    pub image: Option<String>,
    /// Path prefixes whose changes release it.
    pub paths: Vec<String>,
    /// Paths under `paths` that never ship: `dir/` anywhere, `*suffix`, or exact.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Release when its resolved dependency tree changes.
    #[serde(default)]
    pub cargo: Option<CargoTree>,
}

/// One generated `CHANGELOG.md`. Exactly one of `paths` and `unit` is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    /// The file, relative to the root.
    pub file: PathBuf,
    /// Path prefixes; dated sections.
    pub paths: Option<Vec<String>>,
    /// A unit; one section per release.
    pub unit: Option<String>,
}

impl Config {
    /// Loads and validates `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if unreadable, [`Error::Policy`] if invalid.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let invalid = |message: String| Error::Policy {
            path: path.to_path_buf(),
            message,
        };
        let config: Self = serde_saphyr::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        for setting in &config.settings {
            for re in setting.tables.iter().chain(&setting.keys) {
                Regex::new(re).map_err(|e| invalid(format!("setting `{re}`: {e}")))?;
            }
        }
        for tier in &config.tiers {
            match (&tier.paths, &tier.unit) {
                (Some(_), None) => {}
                (None, Some(u)) if config.units.contains_key(u) => {}
                (None, Some(u)) => return Err(invalid(format!("tier names unknown unit `{u}`"))),
                _ => {
                    return Err(invalid(format!(
                        "tier `{}` takes exactly one of `paths` and `unit`",
                        tier.file.display()
                    )));
                }
            }
        }
        Ok(config)
    }
}

/// One cut release.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Released {
    /// The unit.
    pub unit: String,
    /// Its new version.
    pub version: String,
    /// `<unit>/v<version>`.
    pub tag: String,
    /// The image repository, if the unit has one.
    pub image: Option<String>,
    /// The `pigeon` features.
    pub features: String,
}

/// What a run did, or would do.
#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    /// Releases cut.
    pub released: Vec<Released>,
    /// Files written, relative to the root.
    pub written: Vec<PathBuf>,
}

/// The changelog line for a commit.
fn line(repository: &str, commit: &Commit) -> String {
    let pr = Regex::new(r"^(?<title>.+) \(#(?<n>[0-9]+)\)$").expect("valid regex");
    match pr.captures(&commit.subject) {
        Some(c) => format!(
            "- {} ([#{n}]({repository}/pull/{n}))",
            &c["title"],
            n = &c["n"]
        ),
        None => format!(
            "- {} ([{}]({repository}/commit/{}))",
            commit.subject,
            &commit.sha[..7.min(commit.sha.len())],
            commit.sha
        ),
    }
}

/// The text that identifies a commit's line, for idempotence.
fn link_key(line: &str) -> &str {
    line.rfind(" ([").map_or(line, |i| &line[i..])
}

fn touches(commit: &Commit, prefixes: &[String]) -> bool {
    commit
        .paths
        .iter()
        .any(|p| prefixes.iter().any(|prefix| p.starts_with(prefix.as_str())))
}

fn excluded(path: &str, exclude: &[String]) -> bool {
    exclude.iter().any(|pat| {
        if let Some(suffix) = pat.strip_prefix('*') {
            path.ends_with(suffix)
        } else if pat.ends_with('/') {
            path.starts_with(pat.as_str()) || path.contains(&format!("/{pat}"))
        } else {
            path == pat
        }
    })
}

fn touches_shipped(commit: &Commit, prefixes: &[String], exclude: &[String]) -> bool {
    commit.paths.iter().any(|p| {
        !excluded(p, exclude) && prefixes.iter().any(|prefix| p.starts_with(prefix.as_str()))
    })
}

/// The setting lines of `text`.
fn setting_lines(text: &str, setting: &Setting) -> String {
    let tables = setting
        .tables
        .as_deref()
        .map(|r| Regex::new(r).expect("checked on load"));
    let keys = setting
        .keys
        .as_deref()
        .map(|r| Regex::new(r).expect("checked on load"));
    let mut in_table = false;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_table = tables.as_ref().is_some_and(|t| t.is_match(trimmed));
        }
        if in_table || keys.as_ref().is_some_and(|k| k.is_match(trimmed)) {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out
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
    features: &str,
) -> Result<(BTreeSet<String>, Vec<String>), Error> {
    let dir = std::env::temp_dir().join(format!(
        "sentinel-tree-{}-{}",
        std::process::id(),
        SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let io = |source| Error::Io {
        path: dir.clone(),
        source,
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
            cargo.package.clone(),
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
        let mut crates = BTreeSet::new();
        let mut paths = Vec::new();
        for line in out.lines() {
            let line = line.trim().trim_end_matches(" (*)");
            if line.is_empty() {
                continue;
            }
            let name = line.split(' ').next().unwrap_or(line);
            if let Some(i) = line.find(&local) {
                let rel = line[i + local.len()..].trim_end_matches(')');
                crates.insert(format!("{name} (workspace)"));
                if name != cargo.package {
                    paths.push(format!("{rel}/"));
                }
            } else {
                crates.insert(line.to_string());
            }
        }
        paths.sort();
        paths.dedup();
        Ok((crates, paths))
    })();
    let _ = fs::remove_dir_all(&dir);
    result
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
    Ok(git::commits(root, from, to)?
        .into_iter()
        .filter(|c| !c.subject.starts_with(RELEASE_PREFIX))
        .collect())
}

/// The first version line, in TOML (`version = "X.Y.Z"`), YAML (`version: X.Y.Z`)
/// or JSON (`"version": "X.Y.Z"`): the span of `X.Y.Z`, the patch, and `X.Y`.
fn read_version(manifest: &str) -> Option<(usize, usize, usize, String)> {
    let re = Regex::new(
        r#"(?m)(?:^version\s*=\s*"|^version:\s*"?|"version"\s*:\s*")(?<v>(?<major>[0-9]+)\.(?<minor>[0-9]+)\.(?<patch>[0-9]+))"#,
    )
    .expect("valid regex");
    let c = re.captures(manifest)?;
    let v = c.name("v")?;
    Some((
        v.start(),
        v.end(),
        c["patch"].parse().ok()?,
        format!("{}.{}", &c["major"], &c["minor"]),
    ))
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
        source,
    })?;
    let Some((start, end, patch, major_minor)) = read_version(&manifest) else {
        return Err(Error::Policy {
            path: unit.manifest.clone(),
            message: "no `version = \"X.Y.Z\"` line".into(),
        });
    };
    let tag = git::last_tag(root, name)?;
    let Some(last) = &tag else {
        let own = unit
            .manifest
            .parent()
            .map(|d| format!("{}/", d.display()))
            .unwrap_or_default();
        let mut lines: Vec<String> = user_commits(root, None, to)?
            .iter()
            .rev()
            .filter(|c| touches_shipped(c, std::slice::from_ref(&own), &unit.exclude))
            .map(|c| line(&config.repository, c))
            .collect();
        if lines.is_empty()
            && let Some(last) = git::commits(root, None, to)?.last()
        {
            lines.push(line(&config.repository, last));
        }
        let version = format!("{major_minor}.{patch}");
        return Ok(Some((released(name, unit, version), lines)));
    };
    let range = user_commits(root, Some(last), to)?;
    let mut prefixes = unit.paths.clone();
    let mut dependencies_changed = false;
    if let Some(cargo) = &unit.cargo {
        let (now, workspace) = dependency_tree(root, to, cargo, &unit.features)?;
        prefixes.extend(workspace);
        let (before, _) = dependency_tree(root, last, cargo, &unit.features)?;
        dependencies_changed = before != now;
    }
    let settings: Vec<&str> = settings_commits(root, config, &range)
        .into_iter()
        .map(|c| c.sha.as_str())
        .collect();
    let lockfile = dependencies_changed
        .then(|| {
            range
                .iter()
                .find(|c| {
                    c.paths
                        .iter()
                        .any(|p| p.ends_with("Cargo.lock") || p.ends_with("Cargo.toml"))
                })
                .or(range.last())
                .map(|c| c.sha.as_str())
        })
        .flatten();
    let mut lines = Vec::new();
    for c in range.iter().rev() {
        if touches_shipped(c, &prefixes, &unit.exclude) || settings.contains(&c.sha.as_str()) {
            lines.push(line(&config.repository, c));
        } else if lockfile == Some(c.sha.as_str()) {
            let l = line(&config.repository, c);
            let at = l.rfind(" ([").unwrap_or(l.len());
            lines.push(format!("{} (dependencies){}", &l[..at], &l[at..]));
        }
    }
    if lines.is_empty() {
        return Ok(None);
    }
    let version = format!("{major_minor}.{}", patch + 1);
    let mut bumped = manifest.clone();
    bumped.replace_range(start..end, &version);
    files.insert(unit.manifest.clone(), bumped);
    Ok(Some((released(name, unit, version), lines)))
}

fn released(name: &str, unit: &Unit, version: String) -> Released {
    Released {
        unit: name.to_string(),
        tag: format!("{name}/v{version}"),
        version,
        image: unit.image.clone(),
        features: unit.features.clone(),
    }
}

/// Renders every tier, recording the ones that changed in `files`.
fn render_tiers(
    root: &Path,
    config: &Config,
    from: Option<&str>,
    to: &str,
    releases: &[(Released, Vec<String>)],
    files: &mut BTreeMap<PathBuf, String>,
) -> Result<(), Error> {
    let today = git::date(root, to)?;
    let range = user_commits(root, from, to)?;
    let mut history: Option<Vec<Commit>> = None;
    for tier in &config.tiers {
        let existing = fs::read_to_string(root.join(&tier.file)).ok();
        let mut log = existing
            .as_deref()
            .map(Changelog::parse)
            .unwrap_or_default();
        if let Some(prefixes) = &tier.paths {
            let commits = match (&existing, &mut history) {
                (Some(_), _) => &range,
                (None, Some(all)) => &*all,
                (None, slot @ None) => &*slot.insert(user_commits(root, None, to)?),
            };
            for c in commits.iter().filter(|c| touches(c, prefixes)) {
                let l = line(&config.repository, c);
                if !log.mentions(link_key(&l)) {
                    log.add_dated(&c.date, l);
                }
            }
        } else if let Some(unit) = &tier.unit
            && let Some((released, lines)) = releases.iter().find(|(r, _)| &r.unit == unit)
        {
            log.prepend(format!("v{} ({today})", released.version), lines.clone());
        }
        let rendered = log.render();
        if existing.as_deref() != Some(rendered.as_str()) {
            files.insert(tier.file.clone(), rendered);
        }
    }
    Ok(())
}

/// One release tag of a unit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TagStatus {
    /// The unit.
    pub unit: String,
    /// The image repository, if the unit has one.
    pub image: Option<String>,
    /// The `pigeon` features.
    pub features: String,
    /// `<unit>/vX.Y.Z`.
    pub tag: String,
    /// `X.Y.Z`.
    pub version: String,
    /// Whether this is the unit's highest version.
    pub newest: bool,
    /// The unit's release changelog tier, if it has one.
    pub changelog: Option<String>,
}

/// Every release tag of every unit, each unit's newest version first.
///
/// # Errors
///
/// [`Error::Git`] if git fails.
pub fn tags(root: &Path, config: &Config) -> Result<Vec<TagStatus>, Error> {
    let mut rows = Vec::new();
    for (name, unit) in &config.units {
        let prefix = format!("{name}/v");
        for (i, tag) in git::tags(root, name)?.into_iter().enumerate() {
            let Some(version) = tag.strip_prefix(&prefix).map(str::to_string) else {
                continue;
            };
            rows.push(TagStatus {
                unit: name.clone(),
                image: unit.image.clone(),
                features: unit.features.clone(),
                tag,
                version,
                newest: i == 0,
                changelog: config
                    .tiers
                    .iter()
                    .find(|t| t.unit.as_deref() == Some(name.as_str()))
                    .map(|t| t.file.display().to_string()),
            });
        }
    }
    Ok(rows)
}

/// Runs a release over `from..to`; with `dry_run`, writes nothing. Without
/// `from`, the range starts after the last `chore(release):` commit.
///
/// # Errors
///
/// [`Error::Git`] if git fails, [`Error::Io`] if a file cannot be read or
/// written, [`Error::Policy`] if a manifest has no plain `version` line.
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
    render_tiers(root, config, from, to, &releases, &mut files)?;
    if !dry_run {
        for (rel, content) in &files {
            let path = root.join(rel);
            fs::write(&path, content).map_err(|source| Error::Io { path, source })?;
        }
        if let Some(dir) = notes_dir {
            fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            for (released, lines) in &releases {
                let path = dir.join(format!("{}.md", released.unit));
                fs::write(&path, lines.join("\n") + "\n")
                    .map_err(|source| Error::Io { path, source })?;
            }
        }
    }
    Ok(Outcome {
        released: releases.into_iter().map(|(r, _)| r).collect(),
        written: files.into_keys().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Commit, line, read_version};

    fn commit(subject: &str) -> Commit {
        Commit {
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            date: "2026-01-02".into(),
            subject: subject.into(),
            paths: vec![],
        }
    }

    #[test]
    fn lines_link_prs_or_commits() {
        assert_eq!(
            line("https://x", &commit("feat: a (#12)")),
            "- feat: a ([#12](https://x/pull/12))"
        );
        assert_eq!(
            line("https://x", &commit("docs: b")),
            "- docs: b ([0123456](https://x/commit/0123456789abcdef0123456789abcdef01234567))"
        );
    }

    #[test]
    fn reads_the_package_version_line() {
        let m = "[package]\nname = \"x\"\nversion = \"0.1.9\"\n";
        let (start, end, patch, mm) = read_version(m).expect("version");
        assert_eq!((&m[start..end], patch, mm.as_str()), ("0.1.9", 9, "0.1"));
        assert!(read_version("version.workspace = true\n").is_none());
        let yaml = "description: x\nversion: 1.2.3\n";
        let (s, e, p, mm) = read_version(yaml).expect("yaml");
        assert_eq!((&yaml[s..e], p, mm.as_str()), ("1.2.3", 3, "1.2"));
        let json = "{\n  \"x-cobot\": { \"version\": \"0.4.0\" }\n}\n";
        let (s, e, p, _) = read_version(json).expect("json");
        assert_eq!((&json[s..e], p), ("0.4.0", 0));
    }
}
