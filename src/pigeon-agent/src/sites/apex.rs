//! Zone apexes on disk: the `workloads/` layout check and the files core reads, from core's
//! apex; see `.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

pub use crate::core::infra::apex::*;
use crate::core::infra::files::Files;

/// The disk, as core's [`Files`].
#[derive(Debug, Clone, Copy)]
pub struct Disk;

impl Files for Disk {
    fn read(&self, path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    fn dirs(&self, path: &Path) -> Vec<String> {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// The disk, shared, for a [`Site`].
#[must_use]
pub fn disk() -> Arc<dyn Files> {
    Arc::new(Disk)
}

/// Checks one zone directory: only workload directories, each a unit.
fn zone_dir(rel: &str, dir: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !e.path().is_dir() {
            out.push((
                format!("{rel}/{n}"),
                "a zone holds only workload directories".into(),
            ));
        } else if n == APEX {
            let apex = e.path();
            if !apex.join(crate::core::infra::config::CONFIG).is_file() {
                out.push((format!("{rel}/{n}"), "an apex holds config.yaml".into()));
            }
            for own in [crate::core::infra::config::SCHEMA, "modules", "unit.yaml"] {
                if apex.join(own).exists() {
                    out.push((
                        format!("{rel}/{n}/{own}"),
                        format!("the apex uses {CLOUDFLARE}'s schema and modules, never its own"),
                    ));
                }
            }
        } else if !is_name(&n) {
            out.push((
                format!("{rel}/{n}"),
                "a workload's name is lowercase words joined by dashes".into(),
            ));
        } else if !e.path().join(crate::core::infra::config::CONFIG).is_file()
            && !e.path().join("unit.yaml").is_file()
        {
            out.push((
                format!("{rel}/{n}"),
                "a workload directory holds config.yaml or unit.yaml".into(),
            ));
        }
    }
}

/// A token's capabilities as `(permission, on the account, zones)`, sorted.
pub(crate) fn capabilities(token: &Value) -> Vec<(String, bool, Vec<String>)> {
    let mut out: Vec<(String, bool, Vec<String>)> = token["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let mut zones: Vec<String> = c["zones"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            zones.sort();
            (
                c["permission"].as_str().unwrap_or_default().to_string(),
                c["account"] == true,
                zones,
            )
        })
        .collect();
    out.sort();
    out
}

/// Checks that `infra/cloudflare` mints `token` with exactly `want`; `at` is
/// where a missing token is reported.
pub(crate) fn check_token(
    cf: &Value,
    token: &str,
    want: &[(String, bool, Vec<String>)],
    at: String,
    out: &mut Vec<(String, String)>,
) {
    let found = cf["tokens"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"].as_str() == Some(token));
    let mut want = want.to_vec();
    want.sort();
    match found {
        None => out.push((
            at,
            format!("{CLOUDFLARE}/config.yaml needs a `{token}` token"),
        )),
        Some(t) if capabilities(t) != want => {
            let listed: Vec<String> = want
                .iter()
                .map(|(p, account, zones)| {
                    if *account {
                        format!("`{p}` on the account")
                    } else {
                        format!("`{p}` on `{}`", zones.join("`, `"))
                    }
                })
                .collect();
            out.push((
                format!("{CLOUDFLARE}/config.yaml"),
                format!(
                    "`{token}` holds exactly {}, nothing else",
                    listed.join(", ")
                ),
            ));
        }
        Some(_) => {}
    }
}

/// Checks each apex has exactly its `apex-<zone>` token: `DNS Write`, plus the
/// rule permissions where the zone serves a static site.
fn apex_tokens(
    root: &Path,
    cf: &Value,
    apexes: Vec<(String, String)>,
    out: &mut Vec<(String, String)>,
) {
    for (zone, at) in apexes {
        let mut want = vec![(DNS_WRITE.to_string(), false, vec![zone.clone()])];
        if !crate::sites::static_site::sites(root, &zone).is_empty() {
            for p in crate::sites::static_site::APEX_RULE_PERMISSIONS {
                want.push((p.to_string(), false, vec![zone.clone()]));
            }
        }
        check_token(cf, &token_name(&zone), &want, at, out);
    }
}

/// Checks the `workloads/` layout against `infra/cloudflare`'s config; problems are `(path, message)`.
#[must_use]
pub fn layout(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(listing) = fs::read_dir(root.join("workloads")) else {
        return out;
    };
    let cf: Option<Value> = fs::read_to_string(
        root.join(CLOUDFLARE)
            .join(crate::core::infra::config::CONFIG),
    )
    .ok()
    .map(|t| serde_saphyr::from_str(&t).unwrap_or(Value::Null));
    let mut dirs: Vec<String> = listing
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    let Some(cf) = cf else {
        // Without `infra/cloudflare`, a zone directory needs only a zone's name; the run's own
        // token covers its units (2026-11-05-development-pigeon-its-own-instance.md §4).
        for name in &dirs {
            let rel = format!("workloads/{name}");
            if is_zone(name) {
                zone_dir(&rel, &root.join(&rel), &mut out);
            } else {
                out.push((rel, format!("`{name}` is not a zone name")));
            }
        }
        let zones: Vec<&str> = dirs
            .iter()
            .map(String::as_str)
            .filter(|n| is_zone(n))
            .collect();
        crate::sites::static_site::check_buckets(root, &zones, &mut out);
        return out;
    };
    let listed: BTreeSet<&str> = cf["zones"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for name in &dirs {
        let rel = format!("workloads/{name}");
        let dir = root.join(&rel);
        if !is_zone(name) || !listed.contains(name.as_str()) {
            out.push((
                rel,
                format!("`{name}` is not a zone in {CLOUDFLARE}/config.yaml"),
            ));
            continue;
        }
        zone_dir(&rel, &dir, &mut out);
    }
    let apexes: Vec<(String, String)> = dirs
        .iter()
        .filter(|name| listed.contains(name.as_str()))
        .filter(|name| {
            root.join("workloads")
                .join(name)
                .join(APEX)
                .join(crate::core::infra::config::CONFIG)
                .is_file()
        })
        .map(|name| (name.clone(), format!("workloads/{name}/{APEX}/config.yaml")))
        .collect();
    apex_tokens(root, &cf, apexes, &mut out);
    let zones: Vec<&str> = dirs
        .iter()
        .map(String::as_str)
        .filter(|n| listed.contains(n))
        .collect();
    for zone in &zones {
        crate::sites::static_site::check_tokens(root, &cf, zone, &mut out);
    }
    crate::sites::static_site::check_buckets(root, &zones, &mut out);
    out
}
