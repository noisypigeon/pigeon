//! Provider catalogs: `catalog/<provider>/`, falling back to `infra/<provider>/`, and
//! their capabilities; see `.claude/decisions/2026-11-04-development-platform-instance-separation.md` §2–§3.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::unit::Problem;

/// The catalog area under the root.
pub const DIR: &str = "catalog";

/// A catalog's capabilities file.
pub const CAPABILITIES: &str = "capabilities.yaml";

/// The area a catalog falls back to.
const FALLBACK: &str = "infra";

static NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*(\.[a-z0-9]+(-[a-z0-9]+)*)+$").expect("valid regex")
});

static SCOPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+(-[a-z]+)*$").expect("valid regex"));

/// A provider's catalog directory: `infra/<provider>` while it still holds a
/// `schema.json`, else `catalog/<provider>` when that does, else `infra/<provider>`.
#[must_use]
pub fn home(root: &Path, provider: &str) -> PathBuf {
    let local = root.join(FALLBACK).join(provider);
    let moved = root.join(DIR).join(provider);
    if !local.join(crate::config::SCHEMA).is_file() && moved.join(crate::config::SCHEMA).is_file() {
        moved
    } else {
        local
    }
}

/// The catalog an `infra/<provider>` unit at `rel` uses, once its own `schema.json` has
/// moved to `catalog/`.
#[must_use]
pub fn for_unit(root: &Path, rel: &str) -> Option<PathBuf> {
    let provider = rel.strip_prefix("infra/").filter(|p| !p.contains('/'))?;
    let moved = home(root, provider);
    moved.starts_with(root.join(DIR)).then_some(moved)
}

/// A problem for each provider whose `schema.json` is in both `infra/` and `catalog/`.
#[must_use]
pub fn duplicates(root: &Path) -> Vec<Problem> {
    dirs(root)
        .into_iter()
        .filter_map(|d| {
            let provider = d.file_name()?.to_string_lossy().into_owned();
            let local = root.join(FALLBACK).join(&provider).join(crate::config::SCHEMA);
            (d.join(crate::config::SCHEMA).is_file() && local.is_file()).then(|| Problem {
                path: format!("{DIR}/{provider}/{}", crate::config::SCHEMA),
                message: format!(
                    "also in {FALLBACK}/{provider}/; a catalog lives in one place, and {FALLBACK}/ wins until it moves"
                ),
            })
        })
        .collect()
}

/// The `modules/` root of catalog `name`, beside `home`, under `catalog/` or `infra/`.
#[must_use]
pub fn catalog_modules(home: &Path, name: &str) -> PathBuf {
    let area = home.parent().unwrap_or(home);
    let root = area.parent().unwrap_or(area);
    [
        area.join(name),
        root.join(DIR).join(name),
        root.join(FALLBACK).join(name),
    ]
    .into_iter()
    .find(|d| d.join("modules").is_dir())
    .unwrap_or_else(|| root.join(DIR).join(name))
    .join("modules")
}

/// Every directory directly under `catalog/`, sorted.
#[must_use]
pub fn dirs(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(root.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// One capability: provider permissions, minted at one scope.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    /// The provider's permission names.
    pub permissions: Vec<String>,
    /// What it is minted on, such as `zone` or `bucket`.
    pub scope: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    description: String,
    capabilities: BTreeMap<String, Capability>,
}

/// A provider's declared capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    /// The provider, the catalog directory's name.
    pub provider: String,
    /// The capabilities file, from the root.
    pub path: String,
    /// The capabilities, by name.
    pub capabilities: BTreeMap<String, Capability>,
}

/// Every provider's capabilities file, from the location [`home`] picks first; problems for
/// invalid ones and for [`duplicates`].
#[must_use]
pub fn capabilities(root: &Path) -> (Vec<Catalog>, Vec<Problem>) {
    let mut providers: Vec<String> = [DIR, FALLBACK]
        .iter()
        .flat_map(|area| {
            fs::read_dir(root.join(area))
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    providers.sort();
    providers.dedup();
    let mut catalogs = Vec::new();
    let mut problems = Vec::new();
    for provider in providers {
        let active = home(root, &provider);
        let other = if active.starts_with(root.join(DIR)) {
            root.join(FALLBACK).join(&provider)
        } else {
            root.join(DIR).join(&provider)
        };
        let Some(file) = [active, other]
            .into_iter()
            .map(|d| d.join(CAPABILITIES))
            .find(|f| f.is_file())
        else {
            continue;
        };
        let path = file
            .strip_prefix(root)
            .map_or_else(|_| file.display().to_string(), |p| p.display().to_string());
        let mut add = |message: String| {
            problems.push(Problem {
                path: path.clone(),
                message,
            });
        };
        let parsed = fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_saphyr::from_str::<File>(&t).map_err(|e| e.to_string()));
        let parsed = match parsed {
            Ok(p) => p,
            Err(e) => {
                add(e);
                continue;
            }
        };
        if parsed.description.trim().is_empty() {
            add("`description` is empty".into());
        }
        if parsed.capabilities.is_empty() {
            add("declares no capabilities".into());
        }
        for (name, c) in &parsed.capabilities {
            if !NAME.is_match(name) {
                add(format!("`{name}` is not a dotted lowercase name"));
            }
            if !SCOPE.is_match(&c.scope) {
                add(format!(
                    "`{name}`: scope `{}` is not lowercase-dash",
                    c.scope
                ));
            }
            if c.permissions.is_empty() {
                add(format!("`{name}` names no permissions"));
            }
            let mut seen = std::collections::BTreeSet::new();
            for p in &c.permissions {
                if p.trim().is_empty() {
                    add(format!("`{name}` has an empty permission"));
                } else if !seen.insert(p) {
                    add(format!("`{name}` repeats permission `{p}`"));
                }
            }
        }
        catalogs.push(Catalog {
            provider,
            path,
            capabilities: parsed.capabilities,
        });
    }
    problems.extend(duplicates(root));
    (catalogs, problems)
}
