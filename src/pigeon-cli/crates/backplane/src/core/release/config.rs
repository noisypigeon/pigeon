//! `.config/releases.yaml`: release units, changelog tiers and build settings, parsed and checked
//! from its text.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, PathBuf};

use regex::Regex;
use serde::Deserialize;

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
    /// Retired units' changelogs, which the bot removes.
    #[serde(default)]
    pub retired: Vec<PathBuf>,
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
    /// The file's previous path; the bot moves it to `file`.
    #[serde(default)]
    pub from: Option<PathBuf>,
}

impl Config {
    /// Parses and checks the file's `text`.
    ///
    /// # Errors
    ///
    /// What's wrong with it: unparsable, a bad setting pattern, or a tier or retired path that
    /// doesn't fit.
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Self = serde_saphyr::from_str(text).map_err(|e| e.to_string())?;
        for setting in &config.settings {
            for re in setting.tables.iter().chain(&setting.keys) {
                Regex::new(re).map_err(|e| format!("setting `{re}`: {e}"))?;
            }
        }
        let mut seen = BTreeSet::new();
        for tier in &config.tiers {
            if tier.from.as_ref() == Some(&tier.file) {
                return Err(format!(
                    "tier `{}`: `from` equals `file`",
                    tier.file.display()
                ));
            }
            for p in std::iter::once(&tier.file).chain(&tier.from) {
                if !seen.insert(p.clone()) {
                    return Err(format!("`{}` names two tiers", p.display()));
                }
            }
            match (&tier.paths, &tier.unit) {
                (Some(_), None) => {}
                (None, Some(u)) if config.units.contains_key(u) => {}
                (None, Some(u)) => return Err(format!("tier names unknown unit `{u}`")),
                _ => {
                    return Err(format!(
                        "tier `{}` takes exactly one of `paths` and `unit`",
                        tier.file.display()
                    ));
                }
            }
        }
        for path in &config.retired {
            let relative = path.components().all(|c| matches!(c, Component::Normal(_)));
            if !relative || path.file_name().is_none_or(|n| n != "CHANGELOG.md") {
                return Err(format!(
                    "retired `{}` is not a CHANGELOG.md under the root",
                    path.display()
                ));
            }
            if !seen.insert(path.clone()) {
                return Err(format!(
                    "`{}` is both retired and a tier, or retired twice",
                    path.display()
                ));
            }
        }
        Ok(config)
    }

    /// The release tier of unit `name`, if it has one.
    #[must_use]
    pub fn tier_of(&self, name: &str) -> Option<&Tier> {
        self.tiers.iter().find(|t| t.unit.as_deref() == Some(name))
    }
}
