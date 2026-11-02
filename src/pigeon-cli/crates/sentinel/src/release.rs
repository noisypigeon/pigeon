//! Release units and generated changelog tiers; see
//! `.claude/decisions/2026-11-02-release-crate-units-and-images.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
}

/// A separately versioned, separately imaged crate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    /// The crate's `Cargo.toml`, holding its version.
    pub manifest: PathBuf,
    /// The `pigeon` features its image is built with.
    pub features: String,
    /// The image repository.
    pub image: String,
    /// Path prefixes whose changes release it.
    pub paths: Vec<String>,
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
    /// The image repository.
    pub image: String,
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

fn user_commits(root: &Path, from: Option<&str>, to: &str) -> Result<Vec<Commit>, Error> {
    Ok(git::commits(root, from, to)?
        .into_iter()
        .filter(|c| !c.subject.starts_with(RELEASE_PREFIX))
        .collect())
}

fn read_version(manifest: &str) -> Option<(usize, usize, usize, String)> {
    let re = Regex::new(r#"(?m)^version\s*=\s*"([0-9]+)\.([0-9]+)\.([0-9]+)"[ \t]*$"#)
        .expect("valid regex");
    let c = re.captures(manifest)?;
    Some((
        c.get(0)?.start(),
        c.get(0)?.end(),
        c[3].parse().ok()?,
        format!("{}.{}", &c[1], &c[2]),
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
    let touched: Vec<Commit> = user_commits(root, tag.as_deref(), to)?
        .into_iter()
        .filter(|c| touches(c, &unit.paths))
        .collect();
    let version = match (&tag, touched.is_empty()) {
        (None, _) => format!("{major_minor}.{patch}"),
        (Some(_), true) => return Ok(None),
        (Some(_), false) => {
            let version = format!("{major_minor}.{}", patch + 1);
            let mut bumped = manifest.clone();
            bumped.replace_range(start..end, &format!("version = \"{version}\""));
            files.insert(unit.manifest.clone(), bumped);
            version
        }
    };
    let mut lines: Vec<String> = touched
        .iter()
        .rev()
        .map(|c| line(&config.repository, c))
        .collect();
    if lines.is_empty()
        && let Some(last) = git::commits(root, None, to)?.last()
    {
        lines.push(line(&config.repository, last));
    }
    let released = Released {
        unit: name.to_string(),
        tag: format!("{name}/v{version}"),
        version,
        image: unit.image.clone(),
        features: unit.features.clone(),
    };
    Ok(Some((released, lines)))
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

/// A unit and its newest release tag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnitStatus {
    /// The unit.
    pub unit: String,
    /// The image repository.
    pub image: String,
    /// The `pigeon` features.
    pub features: String,
    /// The newest `<unit>/v*` tag, if any.
    pub tag: Option<String>,
    /// That tag's version.
    pub version: Option<String>,
}

/// Every unit with its newest tag.
///
/// # Errors
///
/// [`Error::Git`] if git fails.
pub fn units(root: &Path, config: &Config) -> Result<Vec<UnitStatus>, Error> {
    config
        .units
        .iter()
        .map(|(name, unit)| {
            let tag = git::last_tag(root, name)?;
            let version = tag
                .as_deref()
                .and_then(|t| t.strip_prefix(&format!("{name}/v")))
                .map(str::to_string);
            Ok(UnitStatus {
                unit: name.clone(),
                image: unit.image.clone(),
                features: unit.features.clone(),
                tag,
                version,
            })
        })
        .collect()
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
        assert_eq!(
            (&m[start..end], patch, mm.as_str()),
            ("version = \"0.1.9\"", 9, "0.1")
        );
        assert!(read_version("version.workspace = true\n").is_none());
    }
}
