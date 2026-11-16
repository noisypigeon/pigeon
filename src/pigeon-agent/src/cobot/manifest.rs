//! An instance's manifest v1 read from disk: its root `config.yaml`, the kinds' schemas it names
//! and its content paths' symlinks; the parsing and checks are [`crate::core::infra::manifest`],
//! per `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cobot::moves::Move;

pub use crate::core::infra::manifest::*;

/// A kind's schema path under `platform`.
#[must_use]
pub fn kind_schema(platform: &Path, kind: &str) -> PathBuf {
    let (catalog, name) = kind.split_once('/').unwrap_or((kind, ""));
    platform
        .join(crate::cobot::catalog::DIR)
        .join(catalog)
        .join(KINDS)
        .join(name)
        .join(crate::cobot::config::SCHEMA)
}

/// The catalog directory a kind belongs to, which holds its presets and modules.
#[must_use]
pub fn kind_home(platform: &Path, kind: &str) -> PathBuf {
    let catalog = kind.split_once('/').map_or(kind, |(c, _)| c);
    platform.join(crate::cobot::catalog::DIR).join(catalog)
}

/// Whether one of `rel`'s existing components under `root` is a symlink.
fn reaches_link(root: &Path, rel: &str) -> bool {
    let mut at = root.to_path_buf();
    for part in rel.split('/') {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// The manifest's `moves` at `root`, read without its kinds; empty without a v1 manifest.
#[must_use]
pub fn moves(root: &Path) -> Vec<Move> {
    fs::read_to_string(root.join(FILE))
        .map(|t| moves_of(&t))
        .unwrap_or_default()
}

/// What [`parse`] needs of the manifest at `root`, its kinds read from `platform`; `None` when
/// `root` has no `config.yaml`.
#[must_use]
pub fn disk(root: &Path, platform: &Path) -> Option<Disk> {
    let path = root.join(FILE);
    if !path.exists() {
        return None;
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string());
    let (kinds, contents) = text.as_deref().map(wanted).unwrap_or_default();
    Some(Disk {
        kinds: kinds
            .into_iter()
            .filter_map(|k| {
                Some((
                    k.clone(),
                    fs::read_to_string(kind_schema(platform, &k)).ok()?,
                ))
            })
            .collect(),
        linked: contents
            .into_iter()
            .filter(|c| reaches_link(root, c))
            .collect(),
        text,
    })
}

/// The manifest at `root`, its kinds read from `platform`; `None` when `root` has no
/// `config.yaml`. Problems are `(path, message)`.
#[must_use]
pub fn load(root: &Path, platform: &Path) -> Option<(Manifest, Vec<(String, String)>)> {
    disk(root, platform).map(|d| parse(root, &d))
}
