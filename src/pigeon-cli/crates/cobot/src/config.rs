//! Config units: `config.yaml` validated by `schema.json`, rendered from its
//! `x-cobot` annotations; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §1–§3
//! and `.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md` §7.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::apex::{self, Site};
use crate::generations::{self, Day, Generation};
use crate::providers::Allowlist;
use crate::secrets;

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

/// One `x-cobot` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `$item` or `$item.<path>`.
    Item(Vec<String>),
    /// `$config.<path>`.
    Config(Vec<String>),
    /// `$secret.<key>`.
    Secret(String),
    /// `$generation.not_before` or `$generation.expires_on`.
    Generation(String),
    /// `$handoff.api_token` or `$handoff.zone_id`.
    Handoff(String),
    /// `$credential.<name>.<field>`, a field of a credential the runner supplies.
    Credential {
        /// The credential, named in `x-cobot.credentials`.
        name: String,
        /// Its field, such as `api_token`.
        field: String,
    },
    /// `$module.<block>.<out>`, `$module.<block>[*].<out>` or `$module.<block>[$item.<path>].<out>`.
    Module {
        /// The referenced block.
        block: String,
        /// `None`, `Some(None)` for `[*]`, or `Some(Some(path))` for `[$item.<path>]`.
        index: Option<Option<Vec<String>>>,
        /// The output.
        output: String,
    },
}

fn path(rest: &str) -> Option<Vec<String>> {
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let rest = rest.strip_prefix('.')?;
    let parts: Vec<String> = rest.split('.').map(str::to_string).collect();
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then_some(parts)
}

/// Parses an expression; `None` for an unknown form.
#[must_use]
pub fn parse(text: &str) -> Option<Expr> {
    if let Some(c) = MODULE_REF.captures(text) {
        let index = match c.name("index").map(|i| i.as_str()) {
            None => None,
            Some("*") => Some(None),
            Some(i) => Some(Some(
                path(i.trim_start_matches("$item")).filter(|p| !p.is_empty())?,
            )),
        };
        return Some(Expr::Module {
            block: c["block"].to_string(),
            index,
            output: c["out"].to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("$item") {
        return path(rest).map(Expr::Item);
    }
    if let Some(rest) = text.strip_prefix("$config") {
        return path(rest).filter(|p| !p.is_empty()).map(Expr::Config);
    }
    if let Some(key) = text.strip_prefix("$secret.") {
        return path(&format!(".{key}"))
            .filter(|p| p.len() == 1)
            .map(|p| Expr::Secret(p[0].clone()));
    }
    if let Some(rest) = text.strip_prefix("$credential") {
        return path(rest)
            .filter(|p| p.len() == 2)
            .map(|p| Expr::Credential {
                name: p[0].clone(),
                field: p[1].clone(),
            });
    }
    if let Some(key) = text.strip_prefix("$handoff.")
        && crate::static_site::HANDOFF_KEYS.contains(&key)
    {
        return Some(Expr::Handoff(key.to_string()));
    }
    match text {
        "$generation.not_before" => Some(Expr::Generation("not_before".into())),
        "$generation.expires_on" => Some(Expr::Generation("expires_on".into())),
        _ => None,
    }
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
#[derive(Debug, Clone, Default)]
struct ModuleInfo {
    variables: BTreeSet<String>,
    outputs: BTreeSet<String>,
    resources: Vec<String>,
    tf: String,
}

fn module_info(dir: &Path) -> ModuleInfo {
    let mut info = ModuleInfo::default();
    let Ok(listing) = fs::read_dir(dir) else {
        return info;
    };
    let mut files: Vec<PathBuf> = listing
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tf")))
        .collect();
    files.sort();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        info.variables
            .extend(VARIABLE.captures_iter(&text).map(|c| c[1].to_string()));
        info.outputs
            .extend(OUTPUT.captures_iter(&text).map(|c| c[1].to_string()));
        info.resources
            .extend(RESOURCE.captures_iter(&text).map(|c| c[2].to_string()));
        info.tf.push_str(&text);
        info.tf.push('\n');
    }
    info
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
            let local = crate::catalog::catalog_modules(home, catalog);
            match platform {
                Some(p) if !local.is_dir() => {
                    p.join(crate::catalog::DIR).join(catalog).join("modules")
                }
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
        Some(s) if s.workload.is_some() => schema = composed_schema(&schema, "workload"),
        Some(_) => schema = composed_schema(&schema, "apex"),
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
fn composed_schema(root: &Value, kind: &str) -> Value {
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
            crate::static_site::HANDOFF,
            [crate::static_site::BUCKET, crate::static_site::DOMAINS],
        )
    } else {
        (
            apex::HANDOFF,
            [apex::EXPANDER, crate::static_site::RULESETS],
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

/// Checks a config unit; each problem is `(file, message)`. `infra_keys` are
/// the break-glass keys a workload may never hold.
#[must_use]
pub fn validate(
    unit: &ConfigUnit,
    dir: &Path,
    area: &str,
    allow: &Allowlist,
    infra_keys: &BTreeSet<String>,
    catalogs: &[crate::catalog::Catalog],
) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    check_top(unit, &mut problems);
    check_credentials(unit, catalogs, &mut problems);
    check_secrets(unit, dir, area, infra_keys, &mut problems);
    check_blocks(unit, allow, &mut problems);
    check_outputs(unit, &mut problems);
    check_apex(unit, &mut problems);
    problems
}

/// Checks `x-cobot.credentials` against the catalogs' capabilities, and each
/// `$credential` reference against it.
fn check_credentials(
    unit: &ConfigUnit,
    catalogs: &[crate::catalog::Catalog],
    problems: &mut Vec<(String, String)>,
) {
    let mut add = |m: String| problems.push((SCHEMA.into(), m));
    let x = x_cobot(&unit.schema);
    if let Some(c) = x.get("credentials") {
        let Some(entries) = c.as_object() else {
            add("`x-cobot.credentials` must map names to requests".into());
            return;
        };
        for (name, req) in entries {
            if !BLOCK.is_match(name) {
                add(format!(
                    "credential `{name}` must be lowercase with underscores"
                ));
            }
            let keys: BTreeSet<&str> = req
                .as_object()
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            let (provider, scope) = (req["provider"].as_str(), req["scope"].as_str());
            let caps: Option<Vec<&str>> = req["capabilities"]
                .as_array()
                .filter(|a| !a.is_empty())
                .and_then(|a| a.iter().map(Value::as_str).collect());
            let (Some(provider), Some(scope), Some(caps)) = (provider, scope, caps) else {
                add(format!(
                    "credential `{name}` needs `provider`, `scope` and a non-empty `capabilities` list"
                ));
                continue;
            };
            let known = BTreeSet::from(["capabilities", "provider", "scope", "target"]);
            if !keys.is_superset(&BTreeSet::from(["capabilities", "provider", "scope"]))
                || !keys.is_subset(&known)
            {
                add(format!(
                    "credential `{name}` takes only `provider`, `capabilities`, `scope` and `target`"
                ));
            }
            if let Some(t) = req.get("target")
                && !target_ok(t)
            {
                add(format!(
                    "credential `{name}`: `target` is a name or `$config.<path>`"
                ));
            }
            let Some(catalog) = catalogs.iter().find(|c| c.provider == provider) else {
                add(format!(
                    "credential `{name}`: no catalog declares capabilities for `{provider}`"
                ));
                continue;
            };
            for cap in caps {
                match catalog.capabilities.get(cap) {
                    None => add(format!(
                        "credential `{name}`: `{cap}` is not in {}",
                        catalog.path
                    )),
                    Some(c) if c.scope != scope => add(format!(
                        "credential `{name}`: `{cap}` is minted on `{}`, not `{scope}`",
                        c.scope
                    )),
                    Some(_) => {}
                }
            }
        }
    }
    let declared = declared_credentials(unit);
    for (text, name, field) in credential_refs(unit) {
        if !declared.contains(&name) {
            add(format!("`{text}` is not in x-cobot.credentials"));
        } else if !BLOCK.is_match(&field) {
            add(format!(
                "`{text}`: the field must be lowercase with underscores"
            ));
        }
    }
}

/// Whether a credential's `target` is a literal name or a `$config` reference.
fn target_ok(t: &Value) -> bool {
    t.as_str().is_some_and(|t| {
        !t.is_empty() && (!t.starts_with('$') || matches!(parse(t), Some(Expr::Config(_))))
    })
}

/// The provider of each credential a unit requests, by name.
#[must_use]
pub fn credential_providers(unit: &ConfigUnit) -> BTreeMap<String, String> {
    x_cobot(&unit.schema)["credentials"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(n, r)| Some((n.clone(), r["provider"].as_str()?.to_string())))
        .collect()
}

/// Every `$credential.<name>.<field>` in a unit's providers and block inputs.
fn credential_refs(unit: &ConfigUnit) -> Vec<(String, String, String)> {
    let inputs: Vec<Value> = blocks(&unit.schema)
        .into_iter()
        .flat_map(|b| b.inputs.into_values())
        .collect();
    std::iter::once(&x_cobot(&unit.schema)["providers"])
        .chain(&inputs)
        .flat_map(exprs_in)
        .filter_map(|text| match parse(text) {
            Some(Expr::Credential { name, field }) => Some((text.to_string(), name, field)),
            _ => None,
        })
        .collect()
}

/// The fields a unit references of each credential it requests; one requested but never
/// referenced has none.
#[must_use]
pub fn credential_fields(unit: &ConfigUnit) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = declared_credentials(unit)
        .into_iter()
        .map(|n| (n, BTreeSet::new()))
        .collect();
    for (_, name, field) in credential_refs(unit) {
        if let Some(fields) = out.get_mut(&name) {
            fields.insert(field);
        }
    }
    out
}

fn check_apex(unit: &ConfigUnit, problems: &mut Vec<(String, String)>) {
    let x = x_cobot(&unit.schema);
    let (kind, expanders) = allowed(unit.site.as_ref());
    let handoff = x.get("handoff");
    if handoff.is_some_and(|h| h != kind) {
        problems.push((SCHEMA.into(), format!("`handoff` must be `{kind}`")));
    }
    if handoff.is_some() && unit.site.is_none() {
        problems.push((
            SCHEMA.into(),
            "only an apex or a schema-less workload under workloads/<zone>/ takes a handoff".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    for b in blocks(&unit.schema).iter().filter(|b| b.derived.is_some()) {
        let expander = b.derived.as_deref().unwrap_or_default();
        if !expanders.contains(&expander) {
            problems.push((
                SCHEMA.into(),
                format!(
                    "block `{}`: `expand` must be one of `{}`",
                    b.name,
                    expanders.join("`, `")
                ),
            ));
            continue;
        }
        let Some(site) = &unit.site else {
            problems.push((
                SCHEMA.into(),
                "only an apex or a schema-less workload derives blocks".into(),
            ));
            continue;
        };
        for p in apex::derive(expander, &unit.config, &unit.home, site).1 {
            if seen.insert(p.clone()) {
                problems.push((CONFIG.into(), p));
            }
        }
    }
}

fn check_top(unit: &ConfigUnit, problems: &mut Vec<(String, String)>) {
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    let x = x_cobot(&unit.schema);
    if !x["version"].as_str().is_some_and(|v| SEMVER.is_match(v)) {
        add(SCHEMA, "`x-cobot.version` must be X.Y.Z".into());
    }
    if x.get("local").is_some_and(|l| !l.is_boolean()) {
        add(SCHEMA, "`x-cobot.local` must be true or false".into());
    }
    match jsonschema::validator_for(&unit.schema) {
        Ok(validator) => {
            for e in validator.iter_errors(&unit.config) {
                add(CONFIG, format!("{} at `{}`", e, e.instance_path()));
            }
        }
        Err(e) => add(SCHEMA, format!("not a valid JSON Schema: {e}")),
    }
    let mut strings = Vec::new();
    walk_strings(&unit.config, &mut strings);
    let published = unit
        .site
        .as_ref()
        .filter(|s| s.workload.is_none())
        .map(|s| crate::apex::published_values(&s.composed(&unit.config), &unit.home))
        .unwrap_or_default();
    if strings
        .iter()
        .any(|s| CREDENTIAL.is_match(s) && !published.contains(*s))
    {
        add(
            CONFIG,
            "holds a value that looks like a credential; secrets belong in secrets.config.yaml"
                .into(),
        );
    }
}

fn check_secrets(
    unit: &ConfigUnit,
    dir: &Path,
    area: &str,
    infra_keys: &BTreeSet<String>,
    problems: &mut Vec<(String, String)>,
) {
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    let declared = declared_secrets(unit);
    let secrets_file = dir.join(secrets::FILE);
    if area == "workloads" {
        for k in &declared {
            if infra_keys.contains(k) || k == "provisioner_token" {
                add(
                    secrets::FILE,
                    format!("`{k}` is an infra/ key; a workload never holds break-glass material"),
                );
            }
        }
    }
    if !declared.is_empty() || secrets_file.exists() {
        match secrets::read(&secrets_file) {
            Ok(values) => {
                let present: BTreeSet<String> = values.keys().cloned().collect();
                if present != declared {
                    add(
                        secrets::FILE,
                        format!("keys {present:?} differ from x-cobot.secrets {declared:?}"),
                    );
                }
                for (k, v) in &values {
                    if !secrets::is_encrypted(v) {
                        add(secrets::FILE, format!("`{k}` is not ENC[cobot:v1:…]"));
                    }
                }
            }
            Err(e) => add(secrets::FILE, e.to_string()),
        }
    }
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

/// Checks one block's name and module; the module's info if it exists.
fn check_module(
    b: &Block,
    home: &Path,
    platform: Option<&Path>,
    add: &mut impl FnMut(&str, String),
) -> Option<ModuleInfo> {
    if !BLOCK.is_match(&b.name) {
        add(
            SCHEMA,
            format!("block name `{}` must be lowercase with underscores", b.name),
        );
    }
    let (catalog, name) = split_module(&b.module);
    if !MODULE_NAME.is_match(name) || catalog.is_some_and(|c| !MODULE_NAME.is_match(c)) {
        add(
            SCHEMA,
            format!(
                "block `{}` names module `{}`, which is not a lowercase-dash name under modules/ or <catalog>/",
                b.name, b.module
            ),
        );
        return None;
    }
    let mdir = module_dir(home, platform, &b.module);
    let inside = mdir
        .canonicalize()
        .ok()
        .zip(modules_root(home, platform, &b.module).canonicalize().ok())
        .is_some_and(|(m, root)| m.starts_with(root) && m.is_dir());
    if !inside {
        add(
            SCHEMA,
            format!(
                "block `{}` names missing module `{}`, looked for {}",
                b.name,
                b.module,
                display_module(home, platform, &b.module)
            ),
        );
        return None;
    }
    let info = module_info(&mdir);
    let manifest = fs::read_to_string(mdir.join("module.yaml")).unwrap_or_default();
    if !MODULE_VERSION.is_match(&manifest) {
        add(
            &format!("{}/module.yaml", display_module(home, platform, &b.module)),
            "needs `version: X.Y.Z`".into(),
        );
    }
    let provider_resources = info
        .resources
        .iter()
        .filter(|r| *r != "terraform_data")
        .count();
    let triggers = info.resources.len() - provider_resources;
    if provider_resources > 1 || triggers > 1 || info.resources.is_empty() {
        add(
            &display_module(home, platform, &b.module),
            "a module holds one resource, plus at most one terraform_data trigger".into(),
        );
    }
    Some(info)
}

/// Checks `x-cobot.each`: blocks over another array property's items.
fn check_each(schema: &Value, add: &mut impl FnMut(&str, String)) {
    let Some(each) = x_cobot(schema).get("each") else {
        return;
    };
    let Some(each) = each.as_object() else {
        add(SCHEMA, "`each` must be a map".into());
        return;
    };
    let x = x_cobot(schema);
    let taken = |name: &str| {
        schema["properties"].get(name).is_some()
            || x["singletons"].get(name).is_some()
            || x["derived"].get(name).is_some()
    };
    for (name, b) in each {
        if taken(name) {
            add(
                SCHEMA,
                format!("each block `{name}` reuses a property's or singleton's name"),
            );
        }
        let over = b["over"].as_str().unwrap_or_default();
        if schema["properties"][over]["type"] != "array" {
            add(
                SCHEMA,
                format!("each block `{name}`: `over` must name an array property"),
            );
        }
        if b.get("for_each").is_none() {
            add(SCHEMA, format!("each block `{name}` needs `for_each`"));
        }
        if b.get("rotate").is_some() {
            add(SCHEMA, format!("each block `{name}` cannot rotate"));
        }
    }
}

fn check_blocks(unit: &ConfigUnit, allow: &Allowlist, problems: &mut Vec<(String, String)>) {
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    check_each(&unit.schema, &mut add);
    let x = x_cobot(&unit.schema);
    let declared = declared_secrets(unit);
    let handoff = has_handoff(unit);
    let all = blocks(&unit.schema);
    let mut infos: BTreeMap<String, ModuleInfo> = BTreeMap::new();
    for b in &all {
        if let Some(info) = check_module(b, &unit.home, unit.platform.as_deref(), &mut add) {
            infos.insert(b.name.clone(), info);
        }
    }
    for b in &all {
        let Some(info) = infos.get(&b.name) else {
            continue;
        };
        for (input, value) in &b.inputs {
            if !info.variables.contains(input) {
                add(
                    SCHEMA,
                    format!("block `{}` sets undeclared input `{input}`", b.name),
                );
            }
            check_exprs(
                value,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
        }
        if b.for_each.is_some()
            && let Err(e) = item_keys(b, &unit.config)
        {
            add(CONFIG, e);
        }
        if let Some((p, o)) = &b.rotate {
            check_exprs(
                p,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
            check_exprs(
                o,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
            if let Err(e) = rotation(b, &unit.config) {
                add(CONFIG, e);
            }
        }
    }
    if let Some(providers) = x["providers"].as_object() {
        for (name, cfg) in providers {
            if allow.by_name(name).is_none() {
                add(SCHEMA, format!("provider `{name}` is not in the allowlist"));
            }
            let pseudo = Block {
                name: format!("provider.{name}"),
                module: String::new(),
                over: String::new(),
                for_each: None,
                derived: None,
                rotate: None,
                inputs: Map::new(),
            };
            check_exprs(
                cfg,
                &pseudo,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
        }
    }
}

/// Checks `x-cobot.outputs`: each a `$module` expression.
fn check_outputs(unit: &ConfigUnit, problems: &mut Vec<(String, String)>) {
    let Some(outputs) = x_cobot(&unit.schema).get("outputs") else {
        return;
    };
    let Some(map) = outputs.as_object() else {
        problems.push((SCHEMA.into(), "`outputs` must be a map".into()));
        return;
    };
    let all = blocks(&unit.schema);
    let infos: BTreeMap<String, ModuleInfo> = all
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                module_info(&module_dir(&unit.home, unit.platform.as_deref(), &b.module)),
            )
        })
        .collect();
    let pseudo = Block {
        name: "outputs".into(),
        module: String::new(),
        over: String::new(),
        for_each: None,
        derived: None,
        rotate: None,
        inputs: Map::new(),
    };
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    for (name, v) in map {
        if !BLOCK.is_match(name)
            || !v
                .as_str()
                .and_then(parse)
                .is_some_and(|e| matches!(e, Expr::Module { .. } | Expr::Secret(_)))
        {
            add(
                SCHEMA,
                format!("output `{name}` must be a `$module` or `$secret` expression"),
            );
            continue;
        }
        check_exprs(
            v,
            &pseudo,
            &all,
            &infos,
            &declared_secrets(unit),
            false,
            &unit.config,
            &mut add,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one validation pass over shared context"
)]
fn check_exprs(
    value: &Value,
    block: &Block,
    all: &[Block],
    infos: &BTreeMap<String, ModuleInfo>,
    declared: &BTreeSet<String>,
    handoff: bool,
    config: &Value,
    add: &mut impl FnMut(&str, String),
) {
    for text in exprs_in(value) {
        match parse(text) {
            None => add(SCHEMA, format!("unknown expression `{text}`")),
            Some(Expr::Item(_) | Expr::Generation(_)) if !block.is_list() => {
                add(SCHEMA, format!("`{text}` needs a list block with for_each"));
            }
            Some(Expr::Generation(_)) if block.rotate.is_none() => {
                add(SCHEMA, format!("`{text}` needs a rotating block"));
            }
            Some(Expr::Secret(k)) if !declared.contains(&k) => {
                add(SCHEMA, format!("`{text}` is not in x-cobot.secrets"));
            }
            Some(Expr::Handoff(_)) if !handoff => {
                add(SCHEMA, format!("`{text}` needs `handoff`"));
            }
            Some(Expr::Config(p)) if lookup(config, &p).is_none() => {
                add(
                    SCHEMA,
                    format!("`{text}` names a path absent from config.yaml"),
                );
            }
            Some(Expr::Module {
                block: b,
                output,
                index,
            }) => match infos.get(&b) {
                _ if !all.iter().any(|x| x.name == b) => {
                    add(SCHEMA, format!("`{text}` names an unknown block"));
                }
                _ if all
                    .iter()
                    .any(|x| x.name == b && x.is_list() != index.is_some()) =>
                {
                    add(
                        SCHEMA,
                        format!(
                            "`{text}`: a list block needs `[*]` or `[$item.…]`, a singleton takes no index"
                        ),
                    );
                }
                Some(info) if !info.outputs.contains(&output) => {
                    add(SCHEMA, format!("`{text}`: module has no output `{output}`"));
                }
                _ => {}
            },
            Some(_) => {}
        }
    }
}

/// Escapes template sequences so a literal string is never evaluated.
fn literal(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("${", "$${").replace("%{", "%%{")),
        Value::Array(a) => Value::Array(a.iter().map(literal).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), literal(v))).collect()),
        other => other.clone(),
    }
}

fn lookup<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, p| v.get(p))
}

fn dotted(path: &[String]) -> String {
    path.iter().fold(String::new(), |mut out, p| {
        out.push('.');
        out.push_str(p);
        out
    })
}

/// Renders an expression; `zone` indexes `var.handoff` (`each.key`, `each.value.zone`).
fn render_value(value: &Value, config: &Value, zone: &str) -> Value {
    match value {
        Value::String(s) if s.starts_with('$') => match parse(s) {
            Some(Expr::Item(p)) => Value::String(format!("${{each.value{}}}", dotted(&p))),
            Some(Expr::Config(p)) => literal(lookup(config, &p).unwrap_or(&Value::Null)),
            Some(Expr::Secret(k)) => Value::String(format!("${{var.secrets[\"{k}\"]}}")),
            Some(Expr::Credential { name, field }) => {
                Value::String(format!("${{var.credentials[\"{name}\"].{field}}}"))
            }
            Some(Expr::Generation(f)) => Value::String(format!("${{each.value._generation.{f}}}")),
            Some(Expr::Handoff(k)) if zone.is_empty() => {
                Value::String(format!("${{var.handoff.{k}}}"))
            }
            Some(Expr::Handoff(k)) => Value::String(format!("${{var.handoff[{zone}].{k}}}")),
            Some(Expr::Module {
                block,
                index,
                output,
            }) => Value::String(match index {
                None => format!("${{module.{block}.{output}}}"),
                Some(None) => format!("${{{{ for k, m in module.{block} : k => m.{output} }}}}"),
                Some(Some(p)) => format!("${{module.{block}[each.value{}].{output}}}", dotted(&p)),
            }),
            None => Value::Null,
        },
        Value::Array(a) => Value::Array(a.iter().map(|v| render_value(v, config, zone)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), render_value(v, config, zone)))
                .collect(),
        ),
        other => literal(other),
    }
}

fn number(value: &Value, config: &Value) -> Option<i64> {
    match value {
        Value::String(s) => match parse(s) {
            Some(Expr::Config(p)) => lookup(config, &p).and_then(Value::as_i64),
            _ => None,
        },
        other => other.as_i64(),
    }
}

/// A rotating block's period and overlap, both checked.
fn rotation(block: &Block, config: &Value) -> Result<(i64, i64), String> {
    let Some((period, overlap)) = &block.rotate else {
        return Err(format!("`{}` does not rotate", block.name));
    };
    match (number(period, config), number(overlap, config)) {
        (Some(p), Some(o)) if p >= 1 && o >= 0 => Ok((p, o)),
        _ => Err(format!(
            "`{}` rotation needs a positive integer period_days and a non-negative integer overlap_days",
            block.name
        )),
    }
}

/// Each list item's key, checked to be a non-empty string and unique.
fn item_keys(block: &Block, config: &Value) -> Result<Vec<String>, String> {
    let Some(Expr::Item(path)) = block.for_each.as_deref().and_then(parse) else {
        return Err(format!(
            "`{}`: for_each must be `$item` or `$item.<field>`",
            block.name
        ));
    };
    let Some(items) = config.get(&block.over).and_then(Value::as_array) else {
        return Err(format!("`{}` must be a list", block.over));
    };
    let mut keys = Vec::new();
    for item in items {
        match lookup(item, &path).and_then(Value::as_str) {
            Some(k) if !k.is_empty() && !k.contains('@') => {
                if keys.iter().any(|x| x == k) {
                    return Err(format!("`{}` repeats the key `{k}`", block.name));
                }
                keys.push(k.to_string());
            }
            _ => {
                return Err(format!(
                    "`{}`: every item needs a non-empty string key without `@`",
                    block.name
                ));
            }
        }
    }
    Ok(keys)
}

/// Existing generations in state, by block then item key.
pub type Existing = BTreeMap<String, BTreeMap<String, Vec<Generation>>>;

/// Reads existing rotating instances from `tofu show -json` state output.
#[must_use]
pub fn existing(state_json: &str, rotating: &[String]) -> Existing {
    let mut out = Existing::new();
    let Ok(state) = serde_json::from_str::<Value>(state_json) else {
        return out;
    };
    for m in state["values"]["root_module"]["child_modules"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(address) = m["address"].as_str() else {
            continue;
        };
        for block in rotating {
            let prefix = format!("module.{block}[\"");
            if let Some(key) = address
                .strip_prefix(&prefix)
                .and_then(|k| k.strip_suffix("\"]"))
                && let Some((name, g)) = Generation::parse(key)
            {
                let gens = out
                    .entry(block.clone())
                    .or_default()
                    .entry(name)
                    .or_default();
                if !gens.contains(&g) {
                    gens.push(g);
                }
            }
        }
    }
    out
}

/// Adds the unit's sensitive variables and root outputs to `root`.
fn extras(unit: &ConfigUnit, root: &mut Value) {
    let x = x_cobot(&unit.schema);
    let config = &unit.config;
    let mut variables = Map::new();
    if !declared_secrets(unit).is_empty() {
        variables.insert(
            "secrets".into(),
            json!({ "type": "map(string)", "sensitive": true }),
        );
    }
    if !declared_credentials(unit).is_empty() {
        variables.insert(
            "credentials".into(),
            json!({ "type": "map(map(string))", "sensitive": true }),
        );
    }
    if has_handoff(unit) {
        let shape = if unit.site.as_ref().is_some_and(|s| s.workload.is_some()) {
            "object({ api_token = string, zone_id = string, account_id = string })"
        } else {
            "map(object({ api_token = string, zone_id = string }))"
        };
        variables.insert(
            "handoff".into(),
            json!({ "type": shape, "sensitive": true }),
        );
    }
    if !variables.is_empty() {
        root["variable"] = Value::Object(variables);
    }
    if let Some(outputs) = x["outputs"].as_object() {
        let rendered: Map<String, Value> = outputs
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    json!({ "value": render_value(v, config, ""), "sensitive": true }),
                )
            })
            .collect();
        root["output"] = Value::Object(rendered);
    }
}

/// Required providers, provider blocks, and the names of per-zone providers.
fn providers(
    unit: &ConfigUnit,
    allow: &Allowlist,
) -> (Map<String, Value>, Map<String, Value>, Vec<String>) {
    let config = &unit.config;
    let x = x_cobot(&unit.schema);
    let mut required = Map::new();
    let mut provider = Map::new();
    let mut per_zone: Vec<String> = Vec::new();
    if let Some(providers) = x["providers"].as_object() {
        for (name, cfg) in providers {
            if let Some((source, version)) = allow.by_name(name) {
                required.insert(
                    name.clone(),
                    json!({ "source": source, "version": version }),
                );
            }
            if cfg["per_zone"] == true {
                let mut cfg = cfg.clone();
                if let Some(o) = cfg.as_object_mut() {
                    o.remove("per_zone");
                }
                let mut one = render_value(&cfg, config, "each.key");
                let zones = serde_json::to_string(&apex::zones(config)).unwrap_or_default();
                one["alias"] = json!(ZONE_ALIAS);
                one["for_each"] = Value::String(format!("${{toset({zones})}}"));
                provider.insert(name.clone(), json!([one]));
                per_zone.push(name.clone());
            } else {
                provider.insert(name.clone(), render_value(cfg, config, ""));
            }
        }
    }
    (required, provider, per_zone)
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
    let config = &unit.config;
    let (required, provider, per_zone) = providers(unit, allow);
    let mut modules = Map::new();
    for b in blocks(&unit.schema) {
        let mut block = Map::new();
        block.insert(
            "source".into(),
            Value::String(
                module_dir(&unit.home, unit.platform.as_deref(), &b.module)
                    .display()
                    .to_string(),
            ),
        );
        if let Some(expander) = &b.derived {
            let items = unit
                .site
                .as_ref()
                .map(|site| apex::derive(expander, config, &unit.home, site).0)
                .unwrap_or_default();
            block.insert("for_each".into(), Value::Object(items));
            if !per_zone.is_empty() {
                let mapping: Map<String, Value> = per_zone
                    .iter()
                    .map(|p| {
                        (
                            p.clone(),
                            Value::String(format!("{p}.{ZONE_ALIAS}[each.value.zone]")),
                        )
                    })
                    .collect();
                block.insert("providers".into(), Value::Object(mapping));
            }
        } else if b.for_each.is_some() {
            let mut each = Map::new();
            let keys = item_keys(&b, config).unwrap_or_default();
            let items = config[&b.over].as_array().cloned().unwrap_or_default();
            for (key, item) in keys.into_iter().zip(items.iter()) {
                if b.rotate.is_none() {
                    each.insert(key, literal(item));
                    continue;
                }
                let Ok((period, overlap)) = rotation(&b, config) else {
                    continue;
                };
                let have = existing
                    .get(&b.name)
                    .and_then(|e| e.get(&key))
                    .cloned()
                    .unwrap_or_default();
                for g in generations::instances(today, period, overlap, &have) {
                    let mut value = literal(item);
                    if let Value::Object(o) = &mut value {
                        o.insert(
                            "_generation".into(),
                            json!({
                                "not_before": g.not_before_rfc3339(),
                                "expires_on": g.expires_on_rfc3339(),
                            }),
                        );
                    }
                    each.insert(g.key(&key), value);
                }
            }
            block.insert("for_each".into(), Value::Object(each));
        }
        let workload = unit.site.as_ref().is_some_and(|s| s.workload.is_some());
        let zone = if b.derived.is_some() && !workload {
            "each.value.zone"
        } else {
            ""
        };
        for (k, v) in &b.inputs {
            block.insert(k.clone(), render_value(v, config, zone));
        }
        modules.insert(b.name.clone(), Value::Object(block));
    }
    let mut terraform = json!({
        "required_version": ">= 1.13.0",
        "backend": { "local": { "path": state_file.display().to_string() } }
    });
    if !required.is_empty() {
        terraform["required_providers"] = Value::Object(required);
    }
    let mut root = json!({ "terraform": terraform });
    if !modules.is_empty() {
        root["module"] = Value::Object(modules);
    }
    if !provider.is_empty() {
        root["provider"] = Value::Object(provider);
    }
    extras(unit, &mut root);
    root
}

#[cfg(test)]
mod tests {
    use super::{Expr, parse};

    #[test]
    fn expressions_parse() {
        assert_eq!(parse("$item"), Some(Expr::Item(vec![])));
        assert_eq!(parse("$item.name"), Some(Expr::Item(vec!["name".into()])));
        assert_eq!(
            parse("$config.rotation.period_days"),
            Some(Expr::Config(vec!["rotation".into(), "period_days".into()]))
        );
        assert_eq!(
            parse("$secret.account_id"),
            Some(Expr::Secret("account_id".into()))
        );
        assert_eq!(
            parse("$generation.expires_on"),
            Some(Expr::Generation("expires_on".into()))
        );
        assert_eq!(
            parse("$module.zones[*].id"),
            Some(Expr::Module {
                block: "zones".into(),
                index: Some(None),
                output: "id".into()
            })
        );
        assert_eq!(
            parse("$module.zones[$item.zone].id"),
            Some(Expr::Module {
                block: "zones".into(),
                index: Some(Some(vec!["zone".into()])),
                output: "id".into()
            })
        );
        assert_eq!(
            parse("$module.permission_groups.ids"),
            Some(Expr::Module {
                block: "permission_groups".into(),
                index: None,
                output: "ids".into()
            })
        );
        for bad in [
            "$items",
            "$config",
            "$secret.a.b",
            "$generation.x",
            "$module.Zones.id",
            "$module.z[x].id",
            "$env.HOME",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
