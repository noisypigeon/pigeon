//! Units: discovery and schema v0 validation.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

/// The areas that may hold units.
pub const AREAS: [&str; 2] = ["infra", "workloads"];

const UNIT_FILE: &str = "unit.yaml";

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
    /// Its parsed `unit.yaml`, if it parsed.
    pub spec: Option<Spec>,
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
        if unit_file.is_file() {
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
    units.sort_by(|a, b| a.path.cmp(&b.path));
    (units, problems)
}

/// Every `.tf` file under `dir`, recursively.
fn tf_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(listing) = fs::read_dir(&d) else {
            continue;
        };
        for entry in listing.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "tf") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
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

/// Checks every unit against schema v0; empty means valid.
#[must_use]
pub fn validate(root: &Path) -> Vec<Problem> {
    let (units, mut problems) = discover(root);
    let (name_re, provisioner, functions) = (&*NAME, &*PROVISIONER, &*FUNCTIONS);
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
        let Some(spec) = &unit.spec else {
            continue;
        };
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
        let area = unit.path.split('/').next().unwrap_or_default();
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
            } else if !unit.dir.join(&m.source).is_dir() {
                problems.push(problem(
                    &file,
                    format!("module `{}` source is not a directory", m.name),
                ));
            }
        }
        for tf in tf_files(&unit.dir) {
            let rel = tf
                .strip_prefix(root)
                .map_or_else(|_| tf.display().to_string(), |p| p.display().to_string());
            let Ok(text) = fs::read_to_string(&tf) else {
                problems.push(problem(&rel, "cannot be read"));
                continue;
            };
            if provisioner.is_match(&text) {
                problems.push(problem(&rel, "provisioners are not allowed"));
            }
            if let Some(m) = functions.captures(&text) {
                problems.push(problem(&rel, format!("`{}()` is not allowed", &m[1])));
            }
        }
    }
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
