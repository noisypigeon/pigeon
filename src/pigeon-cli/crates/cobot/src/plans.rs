//! Stored plans: `plans/<id>/` in the state, applied exactly as planned; see the phase 3
//! amendment §5 of `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::s3::hex;
use crate::tofu::Change;
use crate::unit::Unit;

/// The directory under the state holding stored plans.
pub const DIR: &str = "plans";
/// Each plan's manifest.
pub const MANIFEST: &str = "plan.json";
/// Each unit's saved plan file, encrypted like state.
pub const FILE: &str = "plan.tfplan";
/// How long a stored plan may be applied: 24 hours.
pub const LIFETIME_SECS: u64 = 24 * 60 * 60;

/// A stored plan's manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    /// Its ID: hex, from its units, plans and creation time.
    pub id: String,
    /// When it was stored, in Unix seconds.
    pub created: u64,
    /// After this, in Unix seconds, it is refused.
    pub expires: u64,
    /// Its units, in apply order.
    pub units: Vec<StoredUnit>,
}

/// One unit of a stored plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredUnit {
    /// The unit's path.
    pub unit: String,
    /// The SHA-256 of its desired state when planned.
    pub digest: String,
    /// Its planned changes: addresses and actions only.
    pub changes: Vec<Change>,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// The SHA-256 of a unit's desired state, from its rendered `work` directory: its
/// `config.yaml`, its `schema.json` or `unit.yaml`, and every file of each module it renders.
///
/// # Errors
///
/// [`Error::Io`] if a file cannot be read.
pub fn digest(unit: &Unit, work: &Path) -> Result<String, Error> {
    let mut h = Sha256::new();
    let mut add = |label: &str, path: &Path| -> Result<(), Error> {
        let bytes = fs::read(path).map_err(io(path))?;
        h.update((label.len() as u64).to_be_bytes());
        h.update(label.as_bytes());
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(&bytes);
        Ok(())
    };
    let config = unit.dir.join(crate::config::CONFIG);
    if config.is_file() {
        add("config", &config)?;
    }
    match &unit.config {
        Some(c) => add("schema", &c.home.join(crate::config::SCHEMA))?,
        None => add("unit", &unit.dir.join(crate::unit::UNIT_FILE))?,
    }
    let rendered = work.join("main.tf.json");
    let text = fs::read_to_string(&rendered).map_err(io(&rendered))?;
    let root: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::Io {
        path: rendered.clone(),
        message: e.to_string(),
    })?;
    let mut sources: Vec<(String, PathBuf)> = root["module"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, m)| {
            let source = Path::new(m["source"].as_str()?);
            let dir = if source.is_absolute() {
                source.to_path_buf()
            } else {
                unit.dir.join(source)
            };
            Some((name.clone(), dir))
        })
        .collect();
    sources.sort();
    for (name, dir) in sources {
        let mut files = Vec::new();
        module_files(&dir, &dir, &mut files)?;
        files.sort();
        for rel in files {
            add(&format!("module {name} {rel}"), &dir.join(&rel))?;
        }
    }
    Ok(hex(&h.finalize()))
}

/// The files under a module directory, relative to it, skipping `.terraform`.
fn module_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let path = entry.map_err(io(dir))?.path();
        if path.file_name().is_some_and(|n| n == ".terraform") {
            continue;
        }
        if path.is_dir() {
            module_files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Stores each unit's plan file under `<state>/plans/<id>/`, with a manifest.
///
/// # Errors
///
/// [`Error::Io`] if a file cannot be read or written.
pub fn store(
    state: &Path,
    planned: &[(&Unit, PathBuf, Vec<Change>, String)],
    now: u64,
) -> Result<Stored, Error> {
    let mut h = Sha256::new();
    h.update(now.to_be_bytes());
    let mut units = Vec::new();
    let mut bodies = Vec::new();
    for (unit, file, changes, d) in planned {
        let body = fs::read(file).map_err(io(file))?;
        h.update(unit.path.as_bytes());
        h.update(d.as_bytes());
        h.update(&body);
        units.push(StoredUnit {
            unit: unit.path.clone(),
            digest: d.clone(),
            changes: changes.clone(),
        });
        bodies.push(body);
    }
    let id = hex(&h.finalize()[..16]);
    let dir = state.join(DIR).join(&id);
    for (u, body) in units.iter().zip(&bodies) {
        let path = dir.join(&u.unit).join(FILE);
        fs::create_dir_all(path.parent().unwrap_or(&dir)).map_err(io(&dir))?;
        fs::write(&path, body).map_err(io(&path))?;
    }
    let stored = Stored {
        id,
        created: now,
        expires: now + LIFETIME_SECS,
        units,
    };
    let manifest = dir.join(MANIFEST);
    let json = serde_json::to_string_pretty(&stored).map_err(|e| Error::Io {
        path: manifest.clone(),
        message: e.to_string(),
    })?;
    fs::write(&manifest, json + "\n").map_err(io(&manifest))?;
    Ok(stored)
}

/// Loads plan `id`, refusing one that is unknown or expired.
///
/// # Errors
///
/// [`Error::Plan`] for a malformed, unknown or expired ID or manifest.
pub fn load(state: &Path, id: &str, now: u64) -> Result<Stored, Error> {
    if !valid_id(id) {
        return Err(Error::Plan(format!("`{id}` is not a plan ID")));
    }
    let manifest = state.join(DIR).join(id).join(MANIFEST);
    let text =
        fs::read_to_string(&manifest).map_err(|_| Error::Plan(format!("no stored plan {id}")))?;
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|e| Error::Plan(format!("plan {id}'s manifest: {e}")))?;
    if stored.id != id || stored.units.is_empty() {
        return Err(Error::Plan(format!(
            "plan {id}'s manifest does not match it"
        )));
    }
    if now >= stored.expires {
        return Err(Error::Plan(format!("plan {id} expired; plan again")));
    }
    Ok(stored)
}

/// Copies plan `id`'s file for `unit` into place at `to`.
///
/// # Errors
///
/// [`Error::Plan`] if the file is missing; [`Error::Io`] if it cannot be copied.
pub fn restore(state: &Path, id: &str, unit: &str, to: &Path) -> Result<(), Error> {
    let from = state.join(DIR).join(id).join(unit).join(FILE);
    if !from.is_file() {
        return Err(Error::Plan(format!("plan {id} has no plan for {unit}")));
    }
    fs::copy(&from, to).map_err(io(&from))?;
    Ok(())
}

/// The marker of a unit already applied from a plan.
const APPLIED: &str = "applied";

/// Records that `unit` was applied from plan `id`, so a retry skips it.
///
/// # Errors
///
/// [`Error::Io`] if the marker cannot be written.
pub fn mark_applied(state: &Path, id: &str, unit: &str) -> Result<(), Error> {
    let path = state.join(DIR).join(id).join(unit).join(APPLIED);
    fs::write(&path, b"").map_err(io(&path))
}

/// The units of plan `id` already applied.
#[must_use]
pub fn applied(state: &Path, id: &str) -> std::collections::BTreeSet<String> {
    let dir = state.join(DIR).join(id);
    fs::read_to_string(dir.join(MANIFEST))
        .ok()
        .and_then(|t| serde_json::from_str::<Stored>(&t).ok())
        .map(|s| {
            s.units
                .into_iter()
                .map(|u| u.unit)
                .filter(|u| dir.join(u).join(APPLIED).is_file())
                .collect()
        })
        .unwrap_or_default()
}

/// Removes plan `id`, so it is applied once.
///
/// # Errors
///
/// [`Error::Io`] if it cannot be removed.
pub fn remove(state: &Path, id: &str) -> Result<(), Error> {
    let dir = state.join(DIR).join(id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(io(&dir))?;
    }
    Ok(())
}
