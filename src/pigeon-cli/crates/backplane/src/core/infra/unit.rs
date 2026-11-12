//! Units and their schema v0 checks, from what the shell reads; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use crate::core::infra::providers::Allowlist;

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
    pub config: Option<crate::core::infra::config::ConfigUnit>,
    /// Its kind, `<catalog>/<kind>`, for a unit the instance manifest declares.
    pub kind: Option<String>,
}

impl Unit {
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

pub(crate) fn problem(path: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        path: path.into(),
        message: message.into(),
    }
}

/// Whether `name` ends with `suffix`, ignoring ASCII case.
pub(crate) fn suffix(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.is_char_boundary(name.len() - suffix.len())
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Whether `source` is a relative path that stays inside the unit.
pub(crate) fn stays_inside(source: &str) -> bool {
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

/// A `.tf` file's problems at `rel`: provisioners, runner reads and providers outside `allow`.
pub(crate) fn check_tf(rel: &str, text: &str, allow: &Allowlist) -> Vec<Problem> {
    let mut out = Vec::new();
    if PROVISIONER.is_match(text) {
        out.push(problem(rel, "provisioners are not allowed"));
    }
    if let Some(m) = FUNCTIONS.captures(text) {
        out.push(problem(rel, format!("`{}()` is not allowed", &m[1])));
    }
    out.extend(allow.check(text).into_iter().map(|p| problem(rel, p)));
    out
}

/// A problem when a unit other than `infra/cloudflare` declares `x-cobot.apex` or `x-cobot.workload`.
pub(crate) fn misplaced_apex(unit: &Unit) -> Option<Problem> {
    let c = unit.config.as_ref()?;
    let kind = ["apex", "workload"]
        .into_iter()
        .find(|k| c.schema["x-cobot"].get(k).is_some())?;
    (unit.path != crate::cobot::apex::CLOUDFLARE && c.site.is_none()).then(|| {
        problem(
            format!("{}/{}", unit.path, crate::cobot::config::SCHEMA),
            format!(
                "`x-cobot.{kind}` belongs only in {}",
                crate::cobot::apex::CLOUDFLARE
            ),
        )
    })
}

/// A schema v0 unit's `unit.yaml` problems; `dirs` are the module sources the shell found to be
/// directories inside the unit.
pub(crate) fn check_spec(
    unit: &Unit,
    spec: &Spec,
    area: &str,
    dirs: &BTreeSet<String>,
) -> Vec<Problem> {
    let mut problems = Vec::new();
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
        } else if !dirs.contains(&m.source) {
            problems.push(problem(
                &file,
                format!("module `{}` source is not a directory", m.name),
            ));
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
