//! The instance manifest, `.config/instance.yaml`.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// The manifest's path under the root.
pub const MANIFEST: &str = ".config/instance.yaml";

/// The only schema version.
pub const SCHEMA: &str = "v0";

/// One instance of the platform.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    /// What the instance is.
    pub description: String,
    /// The schema version, `v0`.
    pub schema: String,
    /// Its name, `[a-z0-9-]+`.
    pub name: String,
    /// Its repository URL.
    pub repository: String,
    /// Where its configuration lives, relative to the root.
    #[serde(default)]
    pub paths: Paths,
    /// The root it was loaded from.
    #[serde(skip)]
    pub root: PathBuf,
    /// The platform it runs on, when not its own root: catalogs, providers and templates come
    /// from there; see `.claude/decisions/2026-11-05-development-pigeon-its-own-instance.md`
    /// (2026-11-05 amendment).
    #[serde(skip)]
    pub platform: Option<PathBuf>,
}

/// An instance's configuration paths; each is optional.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    /// The policy directory; without it, the instance runs no policies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policies: Option<PathBuf>,
    /// The provider allowlist; without it, the platform's [`DEFAULT_PROVIDERS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<PathBuf>,
    /// The release configuration; without it, the platform's [`DEFAULT_RELEASES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub releases: Option<PathBuf>,
}

/// The platform's provider allowlist, under its root.
pub const DEFAULT_PROVIDERS: &str = ".config/providers.yaml";

/// The platform's release configuration, under its root.
pub const DEFAULT_RELEASES: &str = ".config/releases.yaml";

impl Instance {
    /// Loads and validates `root`'s manifest.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if it cannot be read, [`Error::Invalid`] listing every problem.
    pub fn load(root: &Path) -> Result<Self, Error> {
        Self::load_on(root, None)
    }

    /// Loads and validates `root`'s manifest, for an instance on the platform at `platform`
    /// (`None`: its own root).
    ///
    /// # Errors
    ///
    /// As [`Instance::load`].
    pub fn load_on(root: &Path, platform: Option<&Path>) -> Result<Self, Error> {
        let path = root.join(MANIFEST);
        let text = fs::read_to_string(&path).map_err(|e| Error::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let mut instance: Self = serde_saphyr::from_str(&text)
            .map_err(|e| Error::Invalid(vec![format!("{MANIFEST}: {e}")]))?;
        instance.root = root.to_path_buf();
        instance.platform = platform.map(Path::to_path_buf);
        let problems = instance.problems();
        if problems.is_empty() {
            Ok(instance)
        } else {
            Err(Error::Invalid(problems))
        }
    }

    /// A configured path, joined to the root.
    #[must_use]
    pub fn resolve(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }

    /// The platform's root: [`Instance::platform`], else the instance's own.
    #[must_use]
    pub fn platform_root(&self) -> &Path {
        self.platform.as_deref().unwrap_or(&self.root)
    }

    /// The provider allowlist: the instance's own, else the platform's.
    #[must_use]
    pub fn providers_file(&self) -> PathBuf {
        self.paths.providers.as_ref().map_or_else(
            || self.platform_root().join(DEFAULT_PROVIDERS),
            |p| self.resolve(p),
        )
    }

    /// The directory holding [`Instance::providers_file`] and its lock.
    #[must_use]
    pub fn providers_dir(&self) -> PathBuf {
        let file = self.providers_file();
        file.parent()
            .map_or_else(|| self.root.clone(), Path::to_path_buf)
    }

    /// The release configuration: the instance's own, else the platform's.
    #[must_use]
    pub fn releases_file(&self) -> PathBuf {
        self.paths.releases.as_ref().map_or_else(
            || self.platform_root().join(DEFAULT_RELEASES),
            |p| self.resolve(p),
        )
    }

    /// The instance's policy directory, if it names one.
    #[must_use]
    pub fn policies_dir(&self) -> Option<PathBuf> {
        self.paths.policies.as_ref().map(|p| self.resolve(p))
    }

    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.schema != SCHEMA {
            problems.push(format!(
                "{MANIFEST}: schema `{}` is not `{SCHEMA}`",
                self.schema
            ));
        }
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            problems.push(format!(
                "{MANIFEST}: name `{}` is not [a-z0-9-]+",
                self.name
            ));
        }
        let root = self.root.canonicalize().ok();
        for (key, path) in [
            ("policies", &self.paths.policies),
            ("providers", &self.paths.providers),
            ("releases", &self.paths.releases),
        ]
        .into_iter()
        .filter_map(|(k, p)| p.as_ref().map(|p| (k, p)))
        {
            let shown = path.display();
            if path.is_absolute() {
                problems.push(format!("{MANIFEST}: paths.{key} `{shown}` is absolute"));
            } else if path.components().any(|c| c == Component::ParentDir) {
                problems.push(format!("{MANIFEST}: paths.{key} `{shown}` leaves the root"));
            } else if let Ok(real) = self.resolve(path).canonicalize() {
                if root.as_ref().is_some_and(|root| !real.starts_with(root)) {
                    problems.push(format!("{MANIFEST}: paths.{key} `{shown}` leaves the root"));
                }
            } else {
                problems.push(format!("{MANIFEST}: paths.{key} `{shown}` does not exist"));
            }
        }
        if self
            .paths
            .providers
            .as_ref()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n != "providers.yaml")
        {
            problems.push(format!(
                "{MANIFEST}: paths.providers is not named `providers.yaml`"
            ));
        }
        if self.paths.providers.is_none() && !self.providers_file().is_file() {
            problems.push(format!(
                "{MANIFEST}: no paths.providers, and the platform has no `{DEFAULT_PROVIDERS}`"
            ));
        }
        if let Err(p) = crate::core::ledger::read(&self.root, self.platform_root()) {
            problems.extend(p);
        }
        problems
    }
}
