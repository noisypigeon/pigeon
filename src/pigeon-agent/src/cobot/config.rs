//! Config units, read from disk: `config.yaml`, `schema.json`, the modules' files and the
//! secrets file, which [`crate::core::infra::config`] validates and renders; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §1–§3
//! and `.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md` §7.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cobot::apex::{self, Site};
use crate::cobot::generations::Day;
use crate::cobot::providers::Allowlist;
use crate::cobot::secrets;
use crate::core::infra::config as core;

pub use crate::core::infra::config::{
    CONFIG, ConfigUnit, Disk, Existing, Expr, Module, ModuleInfo, SCHEMA, credential_fields,
    credential_providers, declared_credentials, declared_secrets, existing, from_kind, has_handoff,
    parse, rotating_blocks, secret_keys,
};

/// The `.tf` files of the module in `dir`, read in order; none when it can't be listed.
fn tf_texts(dir: &Path) -> Vec<String> {
    let Ok(listing) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = listing
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tf")))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|f| fs::read_to_string(f).ok())
        .collect()
}

/// A module reference split into its catalog, if any, and its name.
fn split_module(module: &str) -> (Option<&str>, &str) {
    match module.split_once('/') {
        Some((catalog, name)) => (Some(catalog), name),
        None => (None, module),
    }
}

/// Where a unit's own modules live, or catalog `<c>`'s for a `<c>/<name>` reference;
/// see `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.
fn modules_root(home: &Path, platform: Option<&Path>, module: &str) -> PathBuf {
    match split_module(module) {
        (Some(catalog), _) => {
            let local = crate::cobot::catalog::catalog_modules(home, catalog);
            match platform {
                Some(p) if !local.is_dir() => p
                    .join(crate::cobot::catalog::DIR)
                    .join(catalog)
                    .join("modules"),
                _ => local,
            }
        }
        (None, _) => home.join("modules"),
    }
}

/// A module's directory: `<modules root>/<name>`.
fn module_dir(home: &Path, platform: Option<&Path>, module: &str) -> PathBuf {
    modules_root(home, platform, module).join(split_module(module).1)
}

/// Loads `config.yaml` from `dir`, and `schema.json` from `home`; for the apex, its extension.
///
/// # Errors
///
/// A message naming the file that failed to read or parse.
pub fn load(dir: &Path, home: &Path, site: Option<Site>) -> Result<ConfigUnit, String> {
    let config_text = fs::read_to_string(dir.join(CONFIG)).map_err(|e| format!("{CONFIG}: {e}"))?;
    let mut config: Value =
        serde_saphyr::from_str(&config_text).map_err(|e| format!("{CONFIG}: {e}"))?;
    if let Some(site) = site.as_ref().filter(|s| s.workload.is_none()) {
        config = json!({ "zones": { site.zone.clone(): config } });
    }
    let schema_text =
        fs::read_to_string(home.join(SCHEMA)).map_err(|e| format!("{SCHEMA}: {e}"))?;
    let mut schema: Value =
        serde_json::from_str(&schema_text).map_err(|e| format!("{SCHEMA}: {e}"))?;
    match &site {
        Some(s) if s.workload.is_some() => schema = core::composed_schema(&schema, "workload"),
        Some(_) => schema = core::composed_schema(&schema, "apex"),
        None => {}
    }
    Ok(ConfigUnit {
        config,
        schema,
        home: home.to_path_buf(),
        site,
        platform: None,
    })
}

/// A module's path for messages: `modules/<name>`, or `<area>/<catalog>/modules/<name>`.
fn display_module(home: &Path, platform: Option<&Path>, module: &str) -> String {
    match split_module(module) {
        (Some(_), _) => {
            let dir = module_dir(home, platform, module);
            let root = home.parent().and_then(Path::parent).unwrap_or(home);
            dir.strip_prefix(root).unwrap_or(&dir).display().to_string()
        }
        (None, _) => format!("modules/{module}"),
    }
}

/// What `unit`, in `dir` when it has its own directory, reads from disk: its modules' files,
/// its secrets file and its derived blocks' items.
#[must_use]
pub fn disk(unit: &ConfigUnit, dir: Option<&Path>) -> Disk {
    let (modules, derived) = core::reads(unit);
    let home = &unit.home;
    let platform = unit.platform.as_deref();
    let modules = modules
        .into_iter()
        .map(|module| {
            let mdir = module_dir(home, platform, &module);
            let inside = mdir
                .canonicalize()
                .ok()
                .zip(modules_root(home, platform, &module).canonicalize().ok())
                .is_some_and(|(m, root)| m.starts_with(root) && m.is_dir());
            let manifest = if inside {
                fs::read_to_string(mdir.join("module.yaml")).unwrap_or_default()
            } else {
                String::new()
            };
            let found = Module {
                info: core::module_info(&tf_texts(&mdir)),
                manifest,
                inside,
                source: mdir.display().to_string(),
                display: display_module(home, platform, &module),
            };
            (module, found)
        })
        .collect();
    let secrets = dir.and_then(|dir| {
        let file = dir.join(secrets::FILE);
        (!declared_secrets(unit).is_empty() || file.exists())
            .then(|| secrets::read(&file).map_err(|e| e.to_string()))
    });
    let derived: BTreeMap<String, serde_json::Map<String, Value>> = derived
        .into_iter()
        .filter_map(|(name, expander)| {
            let site = unit.site.as_ref()?;
            Some((name, apex::derive(&expander, &unit.config, home, site).0))
        })
        .collect();
    Disk {
        modules,
        secrets,
        derived,
    }
}

/// Checks the config unit in `dir`; each problem is `(file, message)`. `infra_keys` are the
/// break-glass keys a workload may never hold.
#[must_use]
pub fn validate(
    unit: &ConfigUnit,
    dir: &Path,
    area: &str,
    allow: &Allowlist,
    infra_keys: &std::collections::BTreeSet<String>,
    catalogs: &[crate::cobot::catalog::Catalog],
) -> Vec<(String, String)> {
    core::validate(
        unit,
        &disk(unit, Some(dir)),
        area,
        allow,
        infra_keys,
        catalogs,
    )
}

/// Renders `main.tf.json` for a config unit.
#[must_use]
pub fn render(
    unit: &ConfigUnit,
    state_file: &Path,
    allow: &Allowlist,
    today: Day,
    existing: &Existing,
) -> Value {
    let state_file = state_file.display().to_string();
    core::render(unit, &disk(unit, None), &state_file, allow, today, existing)
}
