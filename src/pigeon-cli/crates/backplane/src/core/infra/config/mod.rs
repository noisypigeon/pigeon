//! Config units: `config.yaml` validated by `schema.json`, rendered from its `x-cobot`
//! annotations, from what the shell reads first; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §1–§3,
//! `.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md` §7 and
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::cobot::apex::{self, Site};

mod check;
mod expr;
mod render;

pub use check::{credential_fields, credential_providers, validate};
pub use expr::{Expr, parse};
pub use render::{Existing, existing, render};

/// The config file inside a unit.
pub const CONFIG: &str = "config.yaml";

/// The schema file inside a unit.
pub const SCHEMA: &str = "schema.json";

/// The alias of a provider with one instance per zone.
const ZONE_ALIAS: &str = "zone";

static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").expect("valid regex"));

static MODULE_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\$module\.(?<block>[a-z][a-z0-9_]*)(\[(?<index>\*|\$item(\.[A-Za-z0-9_]+)*)\])?\.(?<out>[A-Za-z0-9_]+)$")
        .expect("valid regex")
});

static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*variable\s+"([A-Za-z0-9_-]+)""#).expect("valid regex"));

static OUTPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*output\s+"([A-Za-z0-9_-]+)""#).expect("valid regex"));

static RESOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*(resource|data)\s+"([A-Za-z0-9_]+)"\s+"[A-Za-z0-9_-]+""#)
        .expect("valid regex")
});

static MODULE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid regex"));

static SEMVER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").expect("valid regex"));

static CREDENTIAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_\-]{32,}$").expect("valid regex"));

static MODULE_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^version:\s*[0-9]+\.[0-9]+\.[0-9]+\s*$").expect("valid regex")
});

/// A parsed config unit.
#[derive(Debug, Clone)]
pub struct ConfigUnit {
    /// `config.yaml`, as JSON.
    pub config: Value,
    /// `schema.json`.
    pub schema: Value,
    /// The directory holding `schema.json` and `modules/`.
    pub home: PathBuf,
    /// Set for an apex: its root and zone.
    pub site: Option<Site>,
    /// The platform root whose `catalog/` serves `<catalog>/<name>` modules the unit's own
    /// tree lacks, when the instance lives apart from it.
    pub platform: Option<PathBuf>,
}

/// One rendered module block.
#[derive(Debug, Clone)]
struct Block {
    name: String,
    module: String,
    /// The property whose items a keyed block iterates: its own, or `x-cobot.each`'s `over`.
    over: String,
    for_each: Option<String>,
    derived: Option<String>,
    rotate: Option<(Value, Value)>,
    inputs: Map<String, Value>,
}

impl Block {
    /// Whether the block has one instance per key.
    fn is_list(&self) -> bool {
        self.for_each.is_some() || self.derived.is_some()
    }
}

/// A module's interface, read from its `.tf` files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Its variables.
    pub variables: BTreeSet<String>,
    /// Its outputs.
    pub outputs: BTreeSet<String>,
    /// Its resources' types.
    pub resources: Vec<String>,
    /// Its `.tf` files, joined.
    pub tf: String,
}

/// A module reference split into its catalog, if any, and its name.
fn split_module(module: &str) -> (Option<&str>, &str) {
    match module.split_once('/') {
        Some((catalog, name)) => (Some(catalog), name),
        None => (None, module),
    }
}

fn x_cobot(schema: &Value) -> &Value {
    &schema["x-cobot"]
}

fn blocks(schema: &Value) -> Vec<Block> {
    let mut out = Vec::new();
    let mut push = |name: &str, x: &Value, from_property: bool, derived: bool| {
        out.push(Block {
            name: name.to_string(),
            module: x["module"].as_str().unwrap_or_default().to_string(),
            over: x["over"].as_str().unwrap_or(name).to_string(),
            for_each: from_property
                .then(|| x["for_each"].as_str().map(str::to_string))
                .flatten(),
            derived: derived.then(|| x["expand"].as_str().unwrap_or_default().to_string()),
            rotate: x
                .get("rotate")
                .map(|r| (r["period_days"].clone(), r["overlap_days"].clone())),
            inputs: x["inputs"].as_object().cloned().unwrap_or_default(),
        });
    };
    if let Some(props) = schema["properties"].as_object() {
        for (name, prop) in props {
            if let Some(x) = prop.get("x-cobot") {
                push(name, x, true, false);
            }
        }
    }
    if let Some(singles) = x_cobot(schema)["singletons"].as_object() {
        for (name, x) in singles {
            push(name, x, false, false);
        }
    }
    if let Some(each) = x_cobot(schema)["each"].as_object() {
        for (name, x) in each {
            push(name, x, true, false);
        }
    }
    if let Some(derived) = x_cobot(schema)["derived"].as_object() {
        for (name, x) in derived {
            push(name, x, false, true);
        }
    }
    out
}

/// The secret keys a unit declares: its schema's list, or what its config references.
#[must_use]
pub fn declared_secrets(unit: &ConfigUnit) -> BTreeSet<String> {
    if x_cobot(&unit.schema)["secrets"] == "referenced" {
        return apex::referenced_secrets(&unit.config);
    }
    secret_keys(&unit.schema)
}

/// The credentials a unit requests, by name; see the phase 3 amendment of
/// `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.
#[must_use]
pub fn declared_credentials(unit: &ConfigUnit) -> BTreeSet<String> {
    x_cobot(&unit.schema)["credentials"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// Whether the unit takes a handoff.
#[must_use]
pub fn has_handoff(unit: &ConfigUnit) -> bool {
    x_cobot(&unit.schema).get("handoff").is_some()
}

/// The secret keys the schema lists.
#[must_use]
pub fn secret_keys(schema: &Value) -> BTreeSet<String> {
    x_cobot(schema)["secrets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// The blocks that rotate, by name.
#[must_use]
pub fn rotating_blocks(schema: &Value) -> Vec<String> {
    blocks(schema)
        .into_iter()
        .filter(|b| b.rotate.is_some())
        .map(|b| b.name)
        .collect()
}

/// A manifest unit: its settings, validated by its kind's `schema`, its modules from `home`.
#[must_use]
pub fn from_kind(config: Value, schema: Value, home: &Path, site: Option<Site>) -> ConfigUnit {
    ConfigUnit {
        config,
        schema,
        home: home.to_path_buf(),
        site,
        platform: None,
    }
}

/// An apex's or a workload's schema: `home`'s `$defs.<kind>`, composed by its `x-cobot.<kind>`.
#[must_use]
pub fn composed_schema(root: &Value, kind: &str) -> Value {
    let mut x = root["x-cobot"][kind].clone();
    if let Some(o) = x.as_object_mut() {
        o.insert("version".into(), root["x-cobot"]["version"].clone());
    }
    json!({
        "$schema": root["$schema"],
        "$defs": root["$defs"],
        "$ref": format!("#/$defs/{kind}"),
        "x-cobot": x,
    })
}

/// The handoff kind and the expanders a unit at `site` may use.
fn allowed(site: Option<&Site>) -> (&'static str, [&'static str; 2]) {
    if site.is_some_and(|s| s.workload.is_some()) {
        (
            crate::cobot::static_site::HANDOFF,
            [
                crate::cobot::static_site::BUCKET,
                crate::cobot::static_site::DOMAINS,
            ],
        )
    } else {
        (
            apex::HANDOFF,
            [apex::EXPANDER, crate::cobot::static_site::RULESETS],
        )
    }
}

fn walk_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|v| walk_strings(v, out)),
        Value::Object(o) => o.values().for_each(|v| walk_strings(v, out)),
        _ => {}
    }
}

fn exprs_in(value: &Value) -> Vec<&str> {
    let mut strings = Vec::new();
    walk_strings(value, &mut strings);
    strings.into_iter().filter(|s| s.starts_with('$')).collect()
}

/// What a config unit's validation and rendering read from disk, which the shell reads first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disk {
    /// Each block's module, by its module reference.
    pub modules: BTreeMap<String, Module>,
    /// The unit's secrets file: none when it is absent and nothing is declared, else its values
    /// or why it isn't readable.
    pub secrets: Option<Result<BTreeMap<String, String>, String>>,
    /// Each derived block's items, by block name.
    pub derived: BTreeMap<String, Map<String, Value>>,
}

/// One module as the shell found it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Module {
    /// Its interface, empty when it can't be read.
    pub info: ModuleInfo,
    /// Its `module.yaml`, empty when it can't be read.
    pub manifest: String,
    /// Whether it is a directory inside its modules root.
    pub inside: bool,
    /// Its directory, as `main.tf.json`'s `source`.
    pub source: String,
    /// Its path for messages.
    pub display: String,
}

/// The interface of a module whose `.tf` files, in order, are `texts`.
#[must_use]
pub fn module_info(texts: &[String]) -> ModuleInfo {
    let mut info = ModuleInfo::default();
    for text in texts {
        info.variables
            .extend(VARIABLE.captures_iter(text).map(|c| c[1].to_string()));
        info.outputs
            .extend(OUTPUT.captures_iter(text).map(|c| c[1].to_string()));
        info.resources
            .extend(RESOURCE.captures_iter(text).map(|c| c[2].to_string()));
        info.tf.push_str(text);
        info.tf.push('\n');
    }
    info
}

/// The module references and derived blocks of `unit`'s schema whose files the shell reads for
/// [`Disk`]: each module named well, and each derived block's name and expander.
#[must_use]
pub fn reads(unit: &ConfigUnit) -> (Vec<String>, Vec<(String, String)>) {
    let all = blocks(&unit.schema);
    let modules = all
        .iter()
        .filter(|b| module_named_well(&b.module))
        .map(|b| b.module.clone())
        .collect();
    let derived = all
        .iter()
        .filter_map(|b| b.derived.clone().map(|e| (b.name.clone(), e)))
        .collect();
    (modules, derived)
}

/// Whether `module` is a lowercase-dash name, or `<catalog>/<name>` of two.
#[must_use]
pub fn module_named_well(module: &str) -> bool {
    let (catalog, name) = split_module(module);
    MODULE_NAME.is_match(name) && catalog.is_none_or(|c| MODULE_NAME.is_match(c))
}

#[cfg(test)]
mod tests;
