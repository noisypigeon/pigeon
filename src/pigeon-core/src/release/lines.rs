//! A release's changelog lines, version bump and tag rows, from the commits, files
//! and tags the shell reads.

use std::collections::BTreeSet;

use regex::Regex;
use serde::Serialize;

use super::config::{Config, Setting, Unit};

/// The subject prefix of the release bot's own commits.
pub const RELEASE_PREFIX: &str = "chore(release):";

/// One first-parent commit.
#[derive(Debug, Clone)]
pub struct Commit {
    /// Full SHA.
    pub sha: String,
    /// UTC commit date, `YYYY-MM-DD`.
    pub date: String,
    /// The subject line.
    pub subject: String,
    /// Paths the commit changed.
    pub paths: Vec<String>,
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
    /// Files written or removed, relative to the root.
    pub written: Vec<std::path::PathBuf>,
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
    /// The one changelog, `Config::changelog`.
    pub changelog: String,
}

/// The changelog line for a commit.
///
/// # Panics
///
/// Never: its pattern is a constant.
#[must_use]
pub fn line(repository: &str, commit: &Commit) -> String {
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
#[must_use]
pub fn link_key(line: &str) -> &str {
    line.rfind(" ([").map_or(line, |i| &line[i..])
}

/// Whether `commit` changed a path under one of `prefixes`.
#[must_use]
pub fn touches(commit: &Commit, prefixes: &[String]) -> bool {
    commit
        .paths
        .iter()
        .any(|p| prefixes.iter().any(|prefix| p.starts_with(prefix.as_str())))
}

/// Whether `path` matches one of `exclude`: `dir/` anywhere, `*suffix`, or exact.
#[must_use]
pub fn excluded(path: &str, exclude: &[String]) -> bool {
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

/// Whether `commit` changed a path under `prefixes` that `exclude` doesn't name.
#[must_use]
pub fn touches_shipped(commit: &Commit, prefixes: &[String], exclude: &[String]) -> bool {
    commit.paths.iter().any(|p| {
        !excluded(p, exclude) && prefixes.iter().any(|prefix| p.starts_with(prefix.as_str()))
    })
}

/// The setting lines of `text`.
///
/// # Panics
///
/// When `setting` holds a pattern [`Config::parse`] would refuse.
#[must_use]
pub fn setting_lines(text: &str, setting: &Setting) -> String {
    let tables = setting
        .tables
        .as_deref()
        .map(|r| Regex::new(r).expect("checked on parse"));
    let keys = setting
        .keys
        .as_deref()
        .map(|r| Regex::new(r).expect("checked on parse"));
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

/// Whether `subject` is the release bot's own commit: `chore(release):`, without a merged
/// change's ` (!n)` or ` (#n)`.
///
/// # Panics
///
/// Never: its pattern is a constant.
#[must_use]
pub fn is_release(subject: &str) -> bool {
    let merged = Regex::new(r" \((?:!|#)[0-9]+\)$").expect("valid regex");
    subject.starts_with(RELEASE_PREFIX) && !merged.is_match(subject)
}

/// `commits` without the release bot's own.
#[must_use]
pub fn user_only(commits: Vec<Commit>) -> Vec<Commit> {
    commits
        .into_iter()
        .filter(|c| !is_release(&c.subject))
        .collect()
}

/// The first version line, in TOML (`version = "X.Y.Z"`), YAML (`version: X.Y.Z`)
/// or JSON (`"version": "X.Y.Z"`): the span of `X.Y.Z`, the patch, and `X.Y`.
///
/// # Panics
///
/// Never: its pattern is a constant.
#[must_use]
pub fn read_version(manifest: &str) -> Option<(usize, usize, usize, String)> {
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

/// The package name in a `Cargo.toml`'s text: `[package]`'s `name`.
#[must_use]
pub fn package_name(manifest: &str) -> Option<String> {
    let table: toml::Value = manifest.parse().ok()?;
    Some(table.get("package")?.get("name")?.as_str()?.to_string())
}

/// Of `manifests`, each a path and its text, the path of the one whose package is `name`.
#[must_use]
pub fn package_manifest<'a>(manifests: &'a [(String, String)], name: &str) -> Option<&'a str> {
    manifests
        .iter()
        .find(|(_, text)| package_name(text).as_deref() == Some(name))
        .map(|(path, _)| path.as_str())
}

/// Of `manifests`, each a path and its text, the directory of the shallowest one with a
/// `[workspace]` table: `.` for the root's. Ties go to the first by path.
#[must_use]
pub fn workspace_dir(manifests: &[(String, String)]) -> Option<String> {
    manifests
        .iter()
        .filter(|(_, text)| text.lines().any(|l| l.trim() == "[workspace]"))
        .map(|(path, _)| path.as_str())
        .min_by_key(|p| (p.matches('/').count(), *p))
        .map(|p| p.strip_suffix("/Cargo.toml").unwrap_or(".").to_string())
}

/// `manifest` with the version at `start..end` replaced by `version`.
#[must_use]
pub fn bumped(manifest: &str, start: usize, end: usize, version: &str) -> String {
    let mut out = manifest.to_string();
    out.replace_range(start..end, version);
    out
}

/// Unit `name`'s release at `version`.
#[must_use]
pub fn released(name: &str, unit: &Unit, version: String) -> Released {
    Released {
        unit: name.to_string(),
        tag: format!("{name}/v{version}"),
        version,
        image: unit.image.clone(),
        features: unit.features.clone(),
    }
}

/// A never-released unit's lines: the user commits, oldest last, that shipped under its crate's
/// own directory; or, when none did, `last`, the newest commit of all.
#[must_use]
pub fn first_lines(
    repository: &str,
    unit: &Unit,
    user: &[Commit],
    last: Option<&Commit>,
) -> Vec<String> {
    let own = unit
        .manifest
        .parent()
        .map(|d| format!("{}/", d.display()))
        .unwrap_or_default();
    let mut lines: Vec<String> = user
        .iter()
        .rev()
        .filter(|c| touches_shipped(c, std::slice::from_ref(&own), &unit.exclude))
        .map(|c| line(repository, c))
        .collect();
    if lines.is_empty()
        && let Some(last) = last
    {
        lines.push(line(repository, last));
    }
    lines
}

/// A released unit's lines over `range`, the user commits since its last tag: each that
/// shipped under `prefixes` or changed a setting (`settings`, by SHA), and, when its dependency
/// tree changed, the commit that most likely changed it, marked `(dependencies)`.
#[must_use]
pub fn next_lines(
    repository: &str,
    unit: &Unit,
    range: &[Commit],
    prefixes: &[String],
    settings: &[&str],
    dependencies_changed: bool,
) -> Vec<String> {
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
        if touches_shipped(c, prefixes, &unit.exclude) || settings.contains(&c.sha.as_str()) {
            lines.push(line(repository, c));
        } else if lockfile == Some(c.sha.as_str()) {
            let l = line(repository, c);
            let at = l.rfind(" ([").unwrap_or(l.len());
            lines.push(format!("{} (dependencies){}", &l[..at], &l[at..]));
        }
    }
    lines
}

/// `cargo tree --prefix none`'s output read: every crate, workspace crates (those whose path
/// follows `local`) by name, and those crates' paths but `package`'s own.
#[must_use]
pub fn tree_of(out: &str, local: &str, package: &str) -> (BTreeSet<String>, Vec<String>) {
    let mut crates = BTreeSet::new();
    let mut paths = Vec::new();
    for line in out.lines() {
        let line = line.trim().trim_end_matches(" (*)");
        if line.is_empty() {
            continue;
        }
        let name = line.split(' ').next().unwrap_or(line);
        if let Some(i) = line.find(local) {
            let rel = line[i + local.len()..].trim_end_matches(')');
            crates.insert(format!("{name} (workspace)"));
            if name != package {
                paths.push(format!("{rel}/"));
            }
        } else {
            crates.insert(line.to_string());
        }
    }
    paths.sort();
    paths.dedup();
    (crates, paths)
}

/// Unit `name`'s rows for its `tags`, newest first as git lists them: those under `<name>/v`,
/// or `<previous>/v` when the unit has a `previous`.
#[must_use]
pub fn tag_rows(config: &Config, name: &str, unit: &Unit, tags: Vec<String>) -> Vec<TagStatus> {
    let prefix = format!("{name}/v");
    let old = unit.previous.as_ref().map(|p| format!("{p}/v"));
    tags.into_iter()
        .enumerate()
        .filter_map(|(i, tag)| {
            let version = tag
                .strip_prefix(&prefix)
                .or_else(|| old.as_ref().and_then(|o| tag.strip_prefix(o.as_str())))?
                .to_string();
            Some(TagStatus {
                unit: name.to_string(),
                image: unit.image.clone(),
                features: unit.features.clone(),
                tag,
                version,
                newest: i == 0,
                changelog: config.changelog.display().to_string(),
            })
        })
        .collect()
}
