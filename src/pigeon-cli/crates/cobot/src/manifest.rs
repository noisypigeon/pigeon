//! An instance's manifest v1: its root `config.yaml` of kind-typed units, per
//! `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::moves::Move;

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

/// The manifest's `moves` at `root`, read without its kinds; empty without a v1 manifest.
#[must_use]
pub fn moves(root: &Path) -> Vec<Move> {
    fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|t| serde_saphyr::from_str::<Value>(&t).ok())
        .filter(|v| v["schema"] == SCHEMA)
        .and_then(|v| serde_json::from_value(v["moves"].clone()).ok())
        .unwrap_or_default()
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

/// A kind's schema path under `platform`.
#[must_use]
pub fn kind_schema(platform: &Path, kind: &str) -> PathBuf {
    let (catalog, name) = kind.split_once('/').unwrap_or((kind, ""));
    platform
        .join(crate::catalog::DIR)
        .join(catalog)
        .join(KINDS)
        .join(name)
        .join(crate::config::SCHEMA)
}

/// The catalog directory a kind belongs to, which holds its presets and modules.
#[must_use]
pub fn kind_home(platform: &Path, kind: &str) -> PathBuf {
    let catalog = kind.split_once('/').map_or(kind, |(c, _)| c);
    platform.join(crate::catalog::DIR).join(catalog)
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
    if expands(crate::apex::EXPANDER) {
        Role::Zone
    } else if expands(crate::static_site::BUCKET) {
        Role::Site
    } else {
        Role::Plain
    }
}

/// The manifest at `root`, its kinds read from `platform`; `None` when `root` has no v1
/// `config.yaml`. Problems are `(path, message)`.
#[must_use]
pub fn load(root: &Path, platform: &Path) -> Option<(Manifest, Vec<(String, String)>)> {
    let text = fs::read_to_string(root.join(FILE)).ok()?;
    let empty = || Manifest {
        root: root.to_path_buf(),
        name: String::new(),
        units: BTreeMap::new(),
        moves: Vec::new(),
    };
    let value: Value = match serde_saphyr::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Some((empty(), vec![(FILE.into(), e.to_string())])),
    };
    if value["schema"] != SCHEMA {
        return None;
    }
    let mut problems = Vec::new();
    let raw: Raw = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => return Some((empty(), vec![(FILE.into(), e.to_string())])),
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
    for (id, entry) in raw.units {
        let at = format!("{FILE}: units.{id}");
        if !is_id(&id) {
            problems.push((
                at,
                "is not a unit ID (`<area>/<name>`, lowercase, no `.` or `..` segment)".into(),
            ));
            continue;
        }
        let Some(fields) = entry.as_object() else {
            problems.push((at, "must be a map with `kind`".into()));
            continue;
        };
        let Some(kind) = fields.get("kind").and_then(Value::as_str) else {
            problems.push((at, "needs `kind: <catalog>/<kind>`".into()));
            continue;
        };
        if !KIND.is_match(kind) {
            problems.push((at, format!("kind `{kind}` is not `<catalog>/<kind>`")));
            continue;
        }
        let path = kind_schema(platform, kind);
        let schema = match fs::read_to_string(&path)
            .map_err(|_| format!("no kind `{kind}` on the platform"))
            .and_then(|t| {
                serde_json::from_str::<Value>(&t).map_err(|e| format!("kind `{kind}`: {e}"))
            }) {
            Ok(s) => s,
            Err(e) => {
                problems.push((at, e));
                continue;
            }
        };
        let settings: Map<String, Value> = fields
            .iter()
            .filter(|(k, _)| k.as_str() != "kind")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        units.insert(
            id,
            Entry {
                kind: kind.to_string(),
                role: role(&schema),
                schema,
                settings: Value::Object(settings),
            },
        );
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
    };
    problems.extend(manifest.problems());
    Some((manifest, problems))
}

impl Manifest {
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

    /// A unit's content directory under the root, from its `content` setting.
    #[must_use]
    pub fn content(&self, id: &str) -> Option<PathBuf> {
        let rel = self.units.get(id)?.settings["content"].as_str()?;
        is_content(rel).then(|| self.root.join(rel))
    }

    fn problems(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut names: BTreeMap<(String, String), &str> = BTreeMap::new();
        for (id, e) in &self.units {
            let at = format!("{FILE}: units.{id}");
            if let Some(c) = e.settings.get("content")
                && !c.as_str().is_some_and(is_content)
            {
                out.push((
                    at.clone(),
                    "`content` is a relative path inside the instance".into(),
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
                    format!("{FILE}: units.{id}"),
                    format!("serves {zone} under the same name as {first}"),
                ));
            }
        }
        out
    }
}
