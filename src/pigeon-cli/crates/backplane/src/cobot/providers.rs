//! The provider allowlist and lockfile read from disk, and the provider mirror; the checks are
//! [`crate::core::infra::providers`]; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §5.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cobot::Error;

pub use crate::core::infra::providers::*;

impl Allowlist {
    /// Loads `<dir>/providers.yaml`; a missing file allows nothing.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the file exists but is unreadable or invalid.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(ALLOWLIST);
        if !path.exists() {
            return Ok(Self::default());
        }
        let bad = |message: String| Error::Io {
            path: path.clone(),
            message,
        };
        let text = fs::read_to_string(&path).map_err(|e| bad(e.to_string()))?;
        Self::parse(&text).map_err(bad)
    }
}

/// The lockfile path, if the providers directory has one.
#[must_use]
pub fn lockfile(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(LOCKFILE);
    path.is_file().then_some(path)
}

/// The allowlisted sources a unit uses: in `required_providers` of the rendered
/// root `main.tf.json`, and of the `.tf` files of each module it sources, local
/// or from `infra/agnostic`; a relative source resolves against `unit_dir`.
#[must_use]
pub fn used<'a>(allow: &'a Allowlist, unit_dir: &Path, rendered: &Path) -> Vec<&'a str> {
    let mut sources = Vec::new();
    if let Ok(text) = fs::read_to_string(rendered) {
        let (declared, modules) = crate::core::infra::providers::rendered(&text);
        sources.extend(declared);
        for dir in modules {
            collect(&unit_dir.join(dir), &mut sources);
        }
    }
    used_of(allow, &sources)
}

fn collect(dir: &Path, sources: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".terraform") {
            continue;
        }
        if path.is_dir() {
            collect(&path, sources);
        } else if path.extension().is_some_and(|e| e == "tf")
            && let Ok(t) = fs::read_to_string(&path)
        {
            sources.extend(declared(&t));
        }
    }
}

/// Fetches each allowlisted provider's package that `store` holds under [`PACKAGES_PREFIX`]
/// into a packed mirror under `root/.cobot/provider-mirror`, and writes the CLI configuration
/// that installs from it: that configuration's path (none when the bucket holds no package),
/// and the sources found. The lock file still checks each package at `tofu init`.
///
/// # Errors
///
/// A message for a failed read or write.
pub fn fetch_mirror(
    store: &crate::cobot::s3::Bucket,
    allow: &Allowlist,
    root: &Path,
    now: u64,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let dir = root.join(".cobot/provider-mirror");
    let mut found = Vec::new();
    for (source, version) in &allow.providers {
        let key = format!("{PACKAGES_PREFIX}{}", package(source, version));
        let Some(bytes) = store.get(&key, now).map_err(|e| e.0)? else {
            continue;
        };
        let path = dir.join(mirror_path(source, version));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        found.push(source.clone());
    }
    if found.is_empty() {
        return Ok((None, found));
    }
    let config = dir.join("tofu.rc");
    std::fs::write(&config, mirror_config(&dir, &found)).map_err(|e| e.to_string())?;
    Ok((Some(config), found))
}
