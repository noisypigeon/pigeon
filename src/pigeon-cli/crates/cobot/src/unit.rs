//! Units: discovery and schema v0 validation.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::providers::Allowlist;
use serde::Deserialize;

/// The areas that may hold units.
pub const AREAS: [&str; 2] = ["infra", "workloads"];

pub(crate) const UNIT_FILE: &str = "unit.yaml";

/// Functions that read the runner's files or environment during a plan.
const FORBIDDEN_FUNCTIONS: &str = r"\b(file|fileexists|fileset|filebase64|filebase64sha256|filebase64sha512|filemd5|filesha1|filesha256|filesha512|templatefile|abspath|pathexpand)\s*\(";

static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid regex"));
static PROVISIONER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*provisioner\s+""#).expect("valid regex"));
static FUNCTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(FORBIDDEN_FUNCTIONS).expect("valid regex"));

/// `unit.yaml`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// The schema version; only `0`.
    pub schema: u32,
    /// The unit's name.
    pub name: String,
    /// `infra` or `workload`.
    pub kind: Kind,
    /// The unit's modules.
    pub modules: Vec<Module>,
}

/// What a unit is for; must match its area.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Under `infra/`.
    Infra,
    /// Under `workloads/`.
    Workload,
}

/// One module call.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    /// Unique within the unit.
    pub name: String,
    /// A relative path to a directory inside the unit.
    pub source: String,
    /// The module's variables.
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
}

/// A discovered unit.
#[derive(Debug, Clone)]
pub struct Unit {
    /// Its path from the repository root, with `/` separators; also its state key.
    pub path: String,
    /// Its directory.
    pub dir: PathBuf,
    /// Its parsed `unit.yaml`, if it is a v0 unit and parsed.
    pub spec: Option<Spec>,
    /// Its parsed `config.yaml` and `schema.json`, if it is a config unit and parsed.
    pub config: Option<crate::config::ConfigUnit>,
    /// Its kind, `<catalog>/<kind>`, for a unit the instance manifest declares.
    pub kind: Option<String>,
}

impl Unit {
    /// Whether this is a config unit.
    #[must_use]
    pub fn is_config(&self) -> bool {
        self.kind.is_some() || self.dir.join(crate::config::CONFIG).is_file()
    }

    /// Whether its schema sets `x-cobot.local`: it runs only when named; see
    /// `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|c| c.schema["x-cobot"]["local"] == serde_json::Value::Bool(true))
    }
}

/// A validation problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The file or directory, from the repository root.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

fn problem(path: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        path: path.into(),
        message: message.into(),
    }
}

/// Every unit under the areas, sorted by path; problems for unreadable files.
#[must_use]
pub fn discover(root: &Path) -> (Vec<Unit>, Vec<Problem>) {
    discover_in(root, root)
}

/// `p` made absolute, so catalog homes, module sources and problem paths agree for every
/// caller; `p` itself when it cannot be.
fn absolute(p: &Path) -> std::path::PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// [`discover`]'s units under the instance at `root`, composing catalogs from `platform`.
#[must_use]
pub fn discover_in(root: &Path, platform: &Path) -> (Vec<Unit>, Vec<Problem>) {
    let (root, platform) = (absolute(root), absolute(platform));
    let (root, platform) = (root.as_path(), platform.as_path());
    let mut units = Vec::new();
    let mut problems = Vec::new();
    let mut stack: Vec<String> = AREAS
        .iter()
        .filter(|a| root.join(a).is_dir())
        .map(|a| (*a).to_string())
        .collect();
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        let unit_file = dir.join(UNIT_FILE);
        let config_file = dir.join(crate::config::CONFIG);
        if unit_file.is_file() && config_file.is_file() {
            problems.push(problem(
                rel.clone(),
                "holds both unit.yaml and config.yaml; a unit is one or the other",
            ));
        } else if config_file.is_file() {
            let config = match dir_config(root, platform, &rel, &dir) {
                Ok(c) => Some(c),
                Err(e) => {
                    problems.push(problem(format!("{rel}/{e}"), "cannot be loaded"));
                    None
                }
            };
            units.push(Unit {
                path: rel.clone(),
                dir: dir.clone(),
                spec: None,
                config,
                kind: None,
            });
        } else if unit_file.is_file() {
            let spec = match fs::read_to_string(&unit_file) {
                Ok(text) => match serde_saphyr::from_str::<Spec>(&text) {
                    Ok(spec) => Some(spec),
                    Err(e) => {
                        problems.push(problem(format!("{rel}/{UNIT_FILE}"), e.to_string()));
                        None
                    }
                },
                Err(e) => {
                    problems.push(problem(format!("{rel}/{UNIT_FILE}"), e.to_string()));
                    None
                }
            };
            units.push(Unit {
                path: rel.clone(),
                dir: dir.clone(),
                spec,
                config: None,
                kind: None,
            });
        }
        match fs::read_dir(&dir) {
            Ok(listing) => {
                for entry in listing.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        stack.push(format!("{rel}/{}", entry.file_name().to_string_lossy()));
                    }
                }
            }
            Err(e) => problems.push(problem(rel.clone(), format!("cannot list: {e}"))),
        }
    }
    manifest_units(root, platform, &mut units, &mut problems);
    units.sort_by(|a, b| a.path.cmp(&b.path));
    (units, problems)
}

/// The config unit in `dir`, at `rel` under the instance `root`: an apex, a schema-less workload,
/// or a unit with its own or its catalog's schema.
fn dir_config(
    root: &Path,
    platform: &Path,
    rel: &str,
    dir: &Path,
) -> Result<crate::config::ConfigUnit, String> {
    let workload = crate::static_site::zone_workload(rel)
        .filter(|_| !dir.join(crate::config::SCHEMA).exists());
    let site = |zone: &str, workload: Option<&str>| crate::apex::Site {
        root: root.to_path_buf(),
        zone: zone.to_string(),
        workload: workload.map(str::to_string),
        manifest: None,
    };
    let (home, site) = match (crate::apex::zone_apex(rel), workload) {
        (Some(zone), _) => (
            crate::catalog::home_in(root, platform, "cloudflare"),
            Some(site(zone, None)),
        ),
        (None, Some((zone, w))) => (
            crate::catalog::home_in(root, platform, "cloudflare"),
            Some(site(zone, Some(w))),
        ),
        (None, None) => (
            crate::catalog::for_unit_in(root, platform, rel).unwrap_or_else(|| dir.to_path_buf()),
            None,
        ),
    };
    let mut c = crate::config::load(dir, &home, site)?;
    if platform != root {
        c.platform = Some(platform.to_path_buf());
    }
    Ok(c)
}

/// The instance manifest's units, per
/// `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md` §1.
fn manifest_units(
    root: &Path,
    platform: &Path,
    units: &mut Vec<Unit>,
    problems: &mut Vec<Problem>,
) {
    use crate::manifest::{self, Role};
    let Some((m, found)) = manifest::load(root, platform) else {
        return;
    };
    problems.extend(found.into_iter().map(|(p, m)| problem(p, m)));
    let m = std::sync::Arc::new(m);
    for (id, e) in &m.units {
        if units.iter().any(|u| &u.path == id) {
            problems.push(problem(
                format!("{}: units.{id}", manifest::FILE),
                "is also a directory unit",
            ));
            continue;
        }
        let zone = e.zone(id).map(str::to_string);
        let site = match (e.role, zone) {
            (Role::Plain, _) | (_, None) => None,
            (role, Some(zone)) => Some(crate::apex::Site {
                root: root.to_path_buf(),
                zone,
                workload: (role == Role::Site).then(|| manifest::name(id).to_string()),
                manifest: Some(m.clone()),
            }),
        };
        let mut config = e.settings.clone();
        if e.role == Role::Zone
            && let Some(o) = config.as_object_mut()
        {
            o.entry("zone").or_insert_with(|| manifest::name(id).into());
        }
        let mut c = crate::config::from_kind(
            config,
            e.schema.clone(),
            &manifest::kind_home(platform, &e.kind),
            site,
        );
        c.platform = Some(platform.to_path_buf());
        units.push(Unit {
            path: id.clone(),
            dir: root.join(id),
            spec: None,
            config: Some(c),
            kind: Some(e.kind.clone()),
        });
    }
}

/// Whether `name` ends with `suffix`, ignoring ASCII case.
fn suffix(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.is_char_boundary(name.len() - suffix.len())
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// The `.tf` files under `dir` to scan, and the paths schema v0 rejects: symlinks
/// and other `OpenTofu` configuration formats.
fn unit_files(dir: &Path) -> (Vec<PathBuf>, Vec<(PathBuf, &'static str)>) {
    let mut tf = Vec::new();
    let mut rejected = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(listing) = fs::read_dir(&d) else {
            rejected.push((d, "cannot be listed"));
            continue;
        };
        for entry in listing.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.file_type() {
                Ok(t) if t.is_symlink() => {
                    rejected.push((path, "symlinks are not allowed in a unit"));
                }
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(_) if suffix(&name, ".tf") => tf.push(path),
                Ok(_)
                    if [".tf.json", ".tofu", ".tofu.json"]
                        .iter()
                        .any(|s| suffix(&name, s)) =>
                {
                    rejected.push((path, "only `.tf` files are allowed in schema v0"));
                }
                Ok(_) => {}
                Err(_) => rejected.push((path, "cannot be inspected")),
            }
        }
    }
    tf.sort();
    rejected.sort();
    (tf, rejected)
}

/// Whether `source` is a relative path that stays inside the unit.
fn stays_inside(source: &str) -> bool {
    let path = Path::new(source);
    if path.is_absolute() {
        return false;
    }
    let mut depth = 0i32;
    for c in path.components() {
        match c {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            _ => return false,
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

/// Checks a directory's files: formats, symlinks, provisioners, runner reads, providers.
fn scan(root: &Path, dir: &Path, allow: &Allowlist, problems: &mut Vec<Problem>) {
    let rel_of = |path: &Path| {
        path.strip_prefix(root)
            .map_or_else(|_| path.display().to_string(), |p| p.display().to_string())
    };
    let (tf_paths, rejected) = unit_files(dir);
    for (path, why) in rejected {
        problems.push(problem(rel_of(&path), why));
    }
    for tf in tf_paths {
        let rel = rel_of(&tf);
        let Ok(text) = fs::read_to_string(&tf) else {
            problems.push(problem(&rel, "cannot be read"));
            continue;
        };
        if PROVISIONER.is_match(&text) {
            problems.push(problem(&rel, "provisioners are not allowed"));
        }
        if let Some(m) = FUNCTIONS.captures(&text) {
            problems.push(problem(&rel, format!("`{}()` is not allowed", &m[1])));
        }
        for p in allow.check(&text) {
            problems.push(problem(&rel, p));
        }
    }
}

/// A problem when a unit other than `infra/cloudflare` declares `x-cobot.apex` or `x-cobot.workload`.
fn misplaced_apex(unit: &Unit) -> Option<Problem> {
    let c = unit.config.as_ref()?;
    let kind = ["apex", "workload"]
        .into_iter()
        .find(|k| c.schema["x-cobot"].get(k).is_some())?;
    (unit.path != crate::apex::CLOUDFLARE && c.site.is_none()).then(|| {
        problem(
            format!("{}/{}", unit.path, crate::config::SCHEMA),
            format!(
                "`x-cobot.{kind}` belongs only in {}",
                crate::apex::CLOUDFLARE
            ),
        )
    })
}

/// Checks a schema v0 unit's `unit.yaml`.
fn check_spec(unit: &Unit, spec: &Spec, area: &str, problems: &mut Vec<Problem>) {
    let file = format!("{}/{UNIT_FILE}", unit.path);
    let name_re = &*NAME;
    if spec.schema != 0 {
        problems.push(problem(
            &file,
            format!("schema {} is unknown; only 0", spec.schema),
        ));
    }
    if !name_re.is_match(&spec.name) {
        problems.push(problem(
            &file,
            format!("name `{}` is not lowercase-dash", spec.name),
        ));
    }
    let expected = if area == "infra" {
        Kind::Infra
    } else {
        Kind::Workload
    };
    if spec.kind != expected {
        problems.push(problem(
            &file,
            format!("kind does not match its area `{area}/`"),
        ));
    }
    if spec.modules.is_empty() {
        problems.push(problem(&file, "declares no modules"));
    }
    let mut names = HashSet::new();
    for m in &spec.modules {
        if !name_re.is_match(&m.name) {
            problems.push(problem(
                &file,
                format!("module name `{}` is not lowercase-dash", m.name),
            ));
        }
        if !names.insert(m.name.as_str()) {
            problems.push(problem(&file, format!("module name `{}` repeats", m.name)));
        }
        if !stays_inside(&m.source) {
            problems.push(problem(
                &file,
                format!("module `{}` source leaves the unit", m.name),
            ));
        } else if !unit
            .dir
            .join(&m.source)
            .canonicalize()
            .is_ok_and(|p| p.is_dir() && unit.dir.canonicalize().is_ok_and(|u| p.starts_with(u)))
        {
            problems.push(problem(
                &file,
                format!("module `{}` source is not a directory", m.name),
            ));
        }
    }
}

/// Checks every unit; empty means valid.
#[must_use]
pub fn validate(root: &Path, allow: &Allowlist) -> Vec<Problem> {
    validate_in(root, root, allow)
}

/// [`validate`] for the instance at `root` on the platform at `platform`.
#[must_use]
pub fn validate_in(root: &Path, platform: &Path, allow: &Allowlist) -> Vec<Problem> {
    validate_absolute(&absolute(root), &absolute(platform), allow)
}

/// [`validate_in`] once both roots are absolute.
fn validate_absolute(root: &Path, platform: &Path, allow: &Allowlist) -> Vec<Problem> {
    let (units, mut problems) = discover_in(root, platform);
    for (path, message) in crate::apex::layout(root) {
        problems.push(problem(path, message));
    }
    for (path, message) in crate::moves::validate(root, &units) {
        problems.push(problem(path, message));
    }
    let catalogs = crate::catalog::capabilities_in(root, platform).0;
    let infra_keys: std::collections::BTreeSet<String> = units
        .iter()
        .filter(|u| u.path.starts_with("infra/"))
        .filter_map(|u| u.config.as_ref())
        .flat_map(crate::config::declared_secrets)
        .collect();
    for (i, unit) in units.iter().enumerate() {
        let file = format!("{}/{UNIT_FILE}", unit.path);
        if let Some(outer) = units[..i]
            .iter()
            .find(|u| unit.path.starts_with(&format!("{}/", u.path)))
        {
            problems.push(problem(
                &file,
                format!("units do not nest; inside {}", outer.path),
            ));
        }
        let area = unit.path.split('/').next().unwrap_or_default();
        problems.extend(misplaced_apex(unit));
        if unit.is_config() {
            if let Some(config) = &unit.config {
                for (file, message) in
                    crate::config::validate(config, &unit.dir, area, allow, &infra_keys, &catalogs)
                {
                    let at = if unit.kind.is_some() {
                        format!("{}: units.{}", crate::manifest::FILE, unit.path)
                    } else {
                        format!("{}/{file}", unit.path)
                    };
                    problems.push(problem(at, message));
                }
            }
            if unit.kind.is_none() {
                scan(root, &unit.dir, allow, &mut problems);
            }
            continue;
        }
        if let Some(spec) = &unit.spec {
            check_spec(unit, spec, area, &mut problems);
        } else {
            continue;
        }
        scan(root, &unit.dir, allow, &mut problems);
    }
    for dir in crate::catalog::dirs(platform) {
        scan(platform, &dir, allow, &mut problems);
    }
    problems.extend(crate::catalog::capabilities_in(root, platform).1);
    problems
}

#[cfg(test)]
mod tests {
    use super::stays_inside;

    #[test]
    fn sources_stay_inside() {
        assert!(stays_inside("./modules/proof"));
        assert!(stays_inside("modules/a/../b"));
        assert!(!stays_inside("../other"));
        assert!(!stays_inside("./a/../../b"));
        assert!(!stays_inside("/abs"));
    }
}
