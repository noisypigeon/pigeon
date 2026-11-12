//! An instance's manifest v1, parsed and checked from what the shell reads: its root
//! `config.yaml`, the kinds' schemas it names and which content paths reach a symlink; per
//! `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md` and
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::cobot::moves::Move;

/// The manifest, under the instance's root.
pub const FILE: &str = "config.yaml";

/// The manifest schema this module reads.
pub const SCHEMA: &str = "v1";

/// Where a kind's schema lives under the platform: `catalog/<catalog>/kinds/<kind>/schema.json`.
pub const KINDS: &str = "kinds";

/// A unit ID: `<area>/<name>[/<name>…]`, never with an empty, `.` or `..` segment.
static ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*(/[a-z0-9]+([.-][a-z0-9]+)*)+$").expect("valid regex")
});

/// A kind's name: `<catalog>/<kind>`.
static KIND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*/[a-z0-9]+(-[a-z0-9]+)*$").expect("valid regex")
});

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[expect(dead_code, reason = "checked before parsing")]
    schema: String,
    name: String,
    description: String,
    repository: String,
    #[serde(default)]
    units: BTreeMap<String, Value>,
    #[serde(default)]
    moves: Vec<Move>,
}

/// What a kind's derived blocks make a unit: a zone's apex, a site, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// It expands a zone's records (`zone-records`).
    Zone,
    /// It expands a site's bucket (`site-bucket`).
    Site,
    /// Neither.
    Plain,
}

/// One unit: its kind, its kind's schema and role, and its settings.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `<catalog>/<kind>`.
    pub kind: String,
    /// The kind's schema, from the platform.
    pub schema: Value,
    /// What the kind makes it.
    pub role: Role,
    /// Its settings: the entry without `kind`.
    pub settings: Value,
    /// The API that applies it, `x-cobot.api`, for a kind cobot applies without `OpenTofu`.
    pub api: Option<String>,
}

impl Entry {
    /// The zone a site serves, from its settings, or a zone unit's own name.
    #[must_use]
    pub fn zone<'a>(&'a self, id: &'a str) -> Option<&'a str> {
        match self.role {
            Role::Zone => Some(name(id)),
            Role::Site => self.settings["zone"].as_str(),
            Role::Plain => None,
        }
    }
}

/// An instance's manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// The instance's root.
    pub root: PathBuf,
    /// Its name.
    pub name: String,
    /// Its units, by ID.
    pub units: BTreeMap<String, Entry>,
    /// Its one-time state moves.
    pub moves: Vec<Move>,
    /// The content paths whose existing components reach a symlink.
    pub linked: BTreeSet<String>,
}

/// A content path: relative, each segment a plain name, never `.` or `..`.
static CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_-][A-Za-z0-9._-]*(/[A-Za-z0-9_-][A-Za-z0-9._-]*)*$")
        .expect("valid regex")
});

/// Whether `p` is a content path.
#[must_use]
pub fn is_content(p: &str) -> bool {
    CONTENT.is_match(p) && !p.split('/').any(|s| s == "." || s == "..")
}

/// A unit ID's last segment: a zone unit's zone, or a site's name.
#[must_use]
pub fn name(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

/// Whether `id` is a valid unit ID.
#[must_use]
pub fn is_id(id: &str) -> bool {
    ID.is_match(id)
}

fn role(schema: &Value) -> Role {
    let derived = &schema["x-cobot"]["derived"];
    let expands = |e: &str| {
        derived
            .as_object()
            .into_iter()
            .flatten()
            .any(|(_, b)| b["expand"] == e)
    };
    if expands(crate::cobot::apex::EXPANDER) {
        Role::Zone
    } else if expands(crate::cobot::static_site::BUCKET) {
        Role::Site
    } else {
        Role::Plain
    }
}

/// What the shell reads for a manifest: its text, the schema text of each kind it names that the
/// platform holds, and which of its content paths reach a symlink.
#[derive(Debug, Clone)]
pub struct Disk {
    /// `config.yaml`, or why it can't be read.
    pub text: Result<String, String>,
    /// Each named kind's `schema.json` text, by `<catalog>/<kind>`; absent when unreadable.
    pub kinds: BTreeMap<String, String>,
    /// The content paths whose existing components reach a symlink.
    pub linked: BTreeSet<String>,
}

impl Default for Disk {
    fn default() -> Self {
        Self {
            text: Err(String::new()),
            kinds: BTreeMap::new(),
            linked: BTreeSet::new(),
        }
    }
}

/// The kinds `text` names, each `<catalog>/<kind>`, and its content paths: what the shell
/// reads next.
#[must_use]
pub fn wanted(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let value: Value = serde_saphyr::from_str(text).unwrap_or_default();
    let mut kinds = BTreeSet::new();
    let mut contents = BTreeSet::new();
    for u in value["units"].as_object().into_iter().flat_map(Map::values) {
        if let Some(k) = u["kind"].as_str().filter(|k| KIND.is_match(k)) {
            kinds.insert(k.to_string());
        }
        if let Some(c) = u["content"].as_str().filter(|c| is_content(c)) {
            contents.insert(c.to_string());
        }
    }
    (kinds, contents)
}

/// The `moves` of a manifest's `text`, read without its kinds; empty unless it is v1.
#[must_use]
pub fn moves_of(text: &str) -> Vec<Move> {
    serde_saphyr::from_str::<Value>(text)
        .ok()
        .filter(|v| v["schema"] == SCHEMA)
        .and_then(|v| serde_json::from_value(v["moves"].clone()).ok())
        .unwrap_or_default()
}

/// One `units.<id>` entry, its kind's schema from `kinds`; a message when it is not one.
fn entry(id: &str, value: &Value, kinds: &BTreeMap<String, String>) -> Result<Entry, String> {
    if !is_id(id) {
        return Err("is not a unit ID (`<area>/<name>`, lowercase, no `.` or `..` segment)".into());
    }
    let fields = value.as_object().ok_or("must be a map with `kind`")?;
    let kind = fields
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("needs `kind: <catalog>/<kind>`")?;
    if !KIND.is_match(kind) {
        return Err(format!("kind `{kind}` is not `<catalog>/<kind>`"));
    }
    let schema: Value = kinds
        .get(kind)
        .ok_or_else(|| format!("no kind `{kind}` on the platform"))
        .and_then(|t| serde_json::from_str(t).map_err(|e| format!("kind `{kind}`: {e}")))?;
    let settings: Map<String, Value> = fields
        .iter()
        .filter(|(k, _)| k.as_str() != "kind")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let settings = Value::Object(settings);
    let api = schema["x-cobot"]["api"].as_str().map(str::to_string);
    if api.is_some() {
        check_settings(&schema, &settings)?;
    }
    Ok(Entry {
        kind: kind.to_string(),
        role: role(&schema),
        schema,
        settings,
        api,
    })
}

/// An API unit's settings against its kind's schema; `OpenTofu` units are checked as config units.
fn check_settings(schema: &Value, settings: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema).map_err(|e| format!("kind schema: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(settings)
        .map(|e| format!("{e} at `{}`", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// The manifest at `root` from what the shell read, `disk`. Problems are `(path, message)`.
#[must_use]
pub fn parse(root: &Path, disk: &Disk) -> (Manifest, Vec<(String, String)>) {
    let empty = || Manifest {
        root: root.to_path_buf(),
        name: String::new(),
        units: BTreeMap::new(),
        moves: Vec::new(),
        linked: BTreeSet::new(),
    };
    let value: Value = match disk
        .text
        .clone()
        .and_then(|t| serde_saphyr::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => return (empty(), vec![(FILE.into(), e)]),
    };
    if value["schema"] != SCHEMA {
        return (
            empty(),
            vec![(
                FILE.into(),
                format!("an instance's root {FILE} is manifest `schema: {SCHEMA}`"),
            )],
        );
    }
    let mut problems = Vec::new();
    let raw: Raw = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => return (empty(), vec![(FILE.into(), e.to_string())]),
    };
    for (field, v) in [
        ("name", &raw.name),
        ("description", &raw.description),
        ("repository", &raw.repository),
    ] {
        if v.trim().is_empty() {
            problems.push((FILE.into(), format!("`{field}` must not be empty")));
        }
    }
    let mut units = BTreeMap::new();
    for (id, value) in raw.units {
        match entry(&id, &value, &disk.kinds) {
            Ok(e) => {
                units.insert(id, e);
            }
            Err(e) => problems.push((format!("{FILE}: units.{id}"), e)),
        }
    }
    for m in &raw.moves {
        for p in [&m.from, &m.to] {
            if !is_id(p) {
                problems.push((format!("{FILE}: moves"), format!("`{p}` is not a unit ID")));
            }
        }
    }
    let manifest = Manifest {
        root: root.to_path_buf(),
        name: raw.name,
        units,
        moves: raw.moves,
        linked: disk.linked.clone(),
    };
    problems.extend(manifest.problems());
    (manifest, problems)
}

impl Manifest {
    /// Whether `rel` is a content path that reaches no symlink.
    fn inside(&self, rel: &str) -> bool {
        is_content(rel) && !self.linked.contains(rel)
    }

    /// The site units serving `zone`, by name.
    #[must_use]
    pub fn sites(&self, zone: &str) -> BTreeMap<String, &Entry> {
        self.units
            .iter()
            .filter(|(id, e)| e.role == Role::Site && e.zone(id) == Some(zone))
            .map(|(id, e)| (name(id).to_string(), e))
            .collect()
    }

    /// The zone unit for `zone`, if the manifest holds one.
    #[must_use]
    pub fn zone(&self, zone: &str) -> Option<&Entry> {
        self.units
            .iter()
            .find(|(id, e)| e.role == Role::Zone && name(id) == zone)
            .map(|(_, e)| e)
    }

    /// A zone unit's apex config: its settings, with `hosts` from the sites that serve it.
    #[must_use]
    pub fn apex_config(&self, zone: &str) -> Value {
        let mut config = self
            .zone(zone)
            .map_or_else(|| Value::Object(Map::new()), |e| e.settings.clone());
        let hosts: Map<String, Value> = self
            .sites(zone)
            .into_iter()
            .map(|(name, e)| (name, e.settings["hosts"].clone()))
            .collect();
        if let Some(o) = config.as_object_mut() {
            o.insert("hosts".into(), Value::Object(hosts));
        }
        config
    }

    /// A unit's content directory under the root, from its `content` setting; `None` unless it
    /// is a content path that reaches no symlink and stays inside the root.
    #[must_use]
    pub fn content(&self, id: &str) -> Option<PathBuf> {
        let rel = self.units.get(id)?.settings["content"].as_str()?;
        self.inside(rel).then(|| self.root.join(rel))
    }

    /// The units applied by `api`, by ID.
    #[must_use]
    pub fn api_units(&self, api: &str) -> Vec<(&str, &Entry)> {
        self.units
            .iter()
            .filter(|(_, e)| e.api.as_deref() == Some(api))
            .map(|(id, e)| (id.as_str(), e))
            .collect()
    }

    fn problems(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut names: BTreeMap<(String, String), &str> = BTreeMap::new();
        let mut zones: BTreeMap<&str, &str> = BTreeMap::new();
        let mut buckets: BTreeMap<String, &str> = BTreeMap::new();
        for (id, e) in &self.units {
            let at = format!("{FILE}: units.{id}");
            if let Some(c) = e.settings.get("content")
                && !c.as_str().is_some_and(|c| self.inside(c))
            {
                out.push((
                    at.clone(),
                    "`content` is a relative path inside the instance, through no symlink".into(),
                ));
            }
            if e.role == Role::Zone
                && let Some(first) = zones.insert(name(id), id)
            {
                out.push((
                    at.clone(),
                    format!("declares zone {} again, after {first}", name(id)),
                ));
            }
            if e.role == Role::Zone
                && let Some(z) = e.settings.get("zone")
                && z != name(id)
            {
                out.push((
                    at.clone(),
                    format!("`zone` is the unit's name, `{}`", name(id)),
                ));
            }
            if e.role != Role::Site {
                continue;
            }
            let Some(zone) = e.zone(id) else { continue };
            if self.zone(zone).is_none() {
                out.push((
                    at.clone(),
                    format!("serves {zone}, which no zone unit in the manifest declares"),
                ));
            }
            if let Some(first) = names.insert((zone.to_string(), name(id).to_string()), id) {
                out.push((
                    at.clone(),
                    format!("serves {zone} under the same name as {first}"),
                ));
            }
            let bucket = crate::cobot::static_site::bucket_name(zone, name(id));
            if bucket.len() > crate::cobot::static_site::BUCKET_MAX {
                out.push((
                    at.clone(),
                    format!(
                        "bucket `{bucket}` is longer than R2's {} characters",
                        crate::cobot::static_site::BUCKET_MAX
                    ),
                ));
            }
            if let Some(first) = buckets.insert(bucket.clone(), id) {
                out.push((at, format!("bucket `{bucket}` is also {first}'s")));
            }
        }
        out
    }
}
