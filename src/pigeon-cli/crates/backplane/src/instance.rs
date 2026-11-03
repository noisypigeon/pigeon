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
    pub paths: Paths,
    /// The root it was loaded from.
    #[serde(skip)]
    pub root: PathBuf,
}

/// An instance's configuration paths.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    /// The policy directory.
    pub policies: PathBuf,
    /// The provider allowlist.
    pub providers: PathBuf,
    /// The release configuration.
    pub releases: PathBuf,
}

impl Instance {
    /// Loads and validates `root`'s manifest.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if it cannot be read, [`Error::Invalid`] listing every problem.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let path = root.join(MANIFEST);
        let text = fs::read_to_string(&path).map_err(|e| Error::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let mut instance: Self = serde_saphyr::from_str(&text)
            .map_err(|e| Error::Invalid(vec![format!("{MANIFEST}: {e}")]))?;
        instance.root = root.to_path_buf();
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
        ] {
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
            .file_name()
            .is_some_and(|n| n != "providers.yaml")
        {
            problems.push(format!(
                "{MANIFEST}: paths.providers is not named `providers.yaml`"
            ));
        }
        problems
    }
}
