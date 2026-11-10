//! Workers: each spec in `infra/<unit>/workers/<name>/worker.yaml`, rendered to a
//! `wrangler.jsonc` and deployed with wrangler from a stored plan; see
//! `.claude/decisions/2026-11-05-development-cobot-workers-area.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::unit::Problem;

/// A spec's file name.
pub const SPEC: &str = "worker.yaml";

/// Where `render` writes each Worker's configuration, from the root; gitignored.
pub const RENDER_DIR: &str = ".cobot/workers";

/// Where stored Worker plans live in the state.
pub const PLANS_DIR: &str = "workers-plans";

/// The wrangler and docker binaries, overridable for tests.
pub const WRANGLER_VAR: &str = "COBOT_WRANGLER";
/// See [`WRANGLER_VAR`].
pub const DOCKER_VAR: &str = "COBOT_DOCKER";

/// The only platform Cloudflare Containers run; every image is built for it, whatever the
/// machine deploying.
pub const PLATFORM: &str = "linux/amd64";

/// Container instance types Cloudflare offers.
const INSTANCE_TYPES: [&str; 6] = [
    "lite",
    "basic",
    "standard-1",
    "standard-2",
    "standard-3",
    "standard-4",
];

/// A Worker, as its spec declares it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// What it is for.
    pub description: String,
    /// Its script name.
    pub name: String,
    /// The crate built into it, from the root, and the features to build with.
    pub entry: Entry,
    /// Its compatibility date, `YYYY-MM-DD`.
    pub compatibility_date: String,
    /// Custom domains it serves.
    #[serde(default)]
    pub routes: Vec<Route>,
    /// Cron Triggers.
    #[serde(default)]
    pub crons: Vec<String>,
    /// Durable Object classes and their migrations.
    #[serde(default)]
    pub durable_objects: DurableObjects,
    /// Container classes, each also a Durable Object class.
    #[serde(default)]
    pub containers: Vec<Container>,
    /// Its bindings.
    #[serde(default)]
    pub bindings: Bindings,
    /// Worker secrets, by name only; `secrets put` sets each.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Whether it also serves its `workers.dev` hostname, e.g. until it has a route.
    #[serde(default)]
    pub workers_dev: bool,
    /// The kind of stack it is deployed into, such as `pigeon/platform`; its values may then
    /// use `$deployment.<setting or output>`, bound by [`bind`]. See
    /// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 3's design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,
}

/// What a value names its stack's settings and outputs by.
pub const DEPLOYMENT_PREFIX: &str = "$deployment.";

pub use crate::ship_wire::DEPLOYMENT_VAR;

fn rebind(v: &mut Value, unit: &str, settings: &Value, bad: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            let Some(rest) = s.strip_prefix(DEPLOYMENT_PREFIX) else {
                return;
            };
            let key = rest.split(['.', '[']).next().unwrap_or_default();
            if settings[key].is_null() {
                *s = format!("$output.{unit}.{rest}");
                return;
            }
            // A nested setting is named by its path, such as `database.name` or `domains[0]`.
            match setting_at(settings, rest) {
                Some(Value::String(x)) => *s = x.clone(),
                Some(Value::Number(n)) => *s = n.to_string(),
                Some(Value::Bool(b)) => *s = b.to_string(),
                Some(_) => bad.push(format!("`{s}`: setting `{rest}` is not a single value")),
                None => bad.push(format!("`{s}`: no setting `{rest}`")),
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|x| rebind(x, unit, settings, bad)),
        Value::Object(m) => m.values_mut().for_each(|x| rebind(x, unit, settings, bad)),
        _ => {}
    }
}

/// The setting at `path` in `settings`: keys joined by `.`, each optionally followed by list
/// indexes, such as `database.name` or `domains[1]`.
fn setting_at<'a>(settings: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(settings, |v, segment| {
        let mut parts = segment.split('[');
        let mut at = v.get(parts.next()?)?;
        for index in parts {
            let i: usize = index.strip_suffix(']')?.parse().ok()?;
            at = at.get(i)?;
        }
        Some(at)
    })
}

/// `found` bound to the stack `unit` of the manifest at `root`: each `$deployment.<setting>`
/// replaced by the unit's setting, each other `$deployment.<output>` by
/// `$output.<unit>.<output>`, and [`DEPLOYMENT_VAR`] set to `unit`. A spec with no
/// `deployment` is returned as it is.
///
/// # Errors
///
/// A message for an unknown unit, one of another kind, or a value that can't be bound.
pub fn bind(root: &Path, found: &Found, unit: &str) -> Result<Found, String> {
    let Some(kind) = found.spec.deployment.as_deref() else {
        return Ok(found.clone());
    };
    let (manifest, _) = crate::manifest::load(root, root)
        .ok_or_else(|| format!("no config.yaml declares stack `{unit}`"))?;
    let entry = manifest
        .units
        .get(unit)
        .ok_or_else(|| format!("config.yaml has no unit `{unit}`"))?;
    if entry.kind != kind {
        return Err(format!("`{unit}` is a `{}`, not a `{kind}`", entry.kind));
    }
    let mut v = serde_json::to_value(&found.spec).map_err(|e| e.to_string())?;
    let mut bad = Vec::new();
    rebind(&mut v, unit, &entry.settings, &mut bad);
    if !bad.is_empty() {
        return Err(bad.join("; "));
    }
    let mut spec: Spec = serde_json::from_value(v).map_err(|e| e.to_string())?;
    spec.deployment = None;
    spec.bindings
        .vars
        .insert(DEPLOYMENT_VAR.to_string(), unit.to_string());
    Ok(Found {
        spec,
        ..found.clone()
    })
}

/// `found` bound as a deploy binds it: to `deployment` when given, else to its kind's only
/// stack; see the step 4c amendment of
/// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`. A spec with no
/// `deployment` is returned as it is.
///
/// # Errors
///
/// A message when no stack is named and its kind has none or several, or as [`bind`].
pub fn bound(root: &Path, found: &Found, deployment: Option<&str>) -> Result<Found, String> {
    let Some(kind) = found.spec.deployment.as_deref() else {
        return Ok(found.clone());
    };
    if let Some(unit) = deployment {
        return bind(root, found, unit);
    }
    match stacks(root, kind).as_slice() {
        [only] => bind(root, found, only),
        [] => Err(format!(
            "deployed into a `{kind}`, but config.yaml has none"
        )),
        _ => Err(format!(
            "deployed into a `{kind}`, and config.yaml has several: pass --deployment <unit>"
        )),
    }
}

/// The units of `kind` in the manifest at `root`, sorted.
#[must_use]
pub fn stacks(root: &Path, kind: &str) -> Vec<String> {
    crate::manifest::load(root, root)
        .map(|(m, _)| {
            m.units
                .into_iter()
                .filter(|(_, e)| e.kind == kind)
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default()
}

/// The crate a Worker is built from, with `worker-build`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The crate's directory, from the root.
    #[serde(rename = "crate")]
    pub krate: String,
    /// Its features, with default features off.
    #[serde(default)]
    pub features: Vec<String>,
}

/// A custom domain.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// The hostname.
    pub custom_domain: String,
}

/// Durable Object classes; each binding is its class's name in `SCREAMING_SNAKE_CASE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DurableObjects {
    /// The classes.
    #[serde(default)]
    pub classes: Vec<String>,
    /// Their migrations, in order.
    #[serde(default)]
    pub migrations: Vec<Migration>,
}

/// One Durable Object migration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    /// Its tag.
    pub tag: String,
    /// Classes it creates, SQLite-backed.
    #[serde(default)]
    pub new_sqlite_classes: Vec<String>,
}

/// A Container class.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    /// Its class.
    pub class: String,
    /// The image, built at deploy.
    pub image: Image,
    /// Its instance type.
    pub instance_type: String,
    /// At most this many instances.
    pub max_instances: u32,
}

/// An image built from a Dockerfile stage, with its directory as the build context.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    /// The Dockerfile, from the root.
    pub dockerfile: String,
    /// The stage.
    pub target: String,
    /// Build arguments, e.g. `FEATURES`.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

/// A Worker's bindings, by binding name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    /// D1 databases.
    #[serde(default)]
    pub d1: BTreeMap<String, D1>,
    /// R2 buckets, each a `$output` reference to its name.
    #[serde(default)]
    pub r2: BTreeMap<String, String>,
    /// Secrets Store secrets.
    #[serde(default)]
    pub secrets_store: BTreeMap<String, StoreSecret>,
    /// Queues.
    #[serde(default)]
    pub queues: Queues,
    /// Plain-text variables; never an ID or a secret.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// The binding that names the running version, for health checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_metadata: Option<String>,
}

/// A D1 database.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct D1 {
    /// Its name.
    pub name: String,
    /// A `$output` reference to its ID.
    pub id: String,
    /// Its migrations directory, from the root, applied at deploy.
    #[serde(default)]
    pub migrations: Option<String>,
}

/// A secret in a Secrets Store.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSecret {
    /// A `$output` reference to the store's ID.
    pub store: String,
    /// The secret's name in the store.
    pub secret: String,
}

/// Queue producers and consumers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Queues {
    /// Binding name to queue name.
    #[serde(default)]
    pub producers: BTreeMap<String, String>,
    /// Queues this Worker consumes.
    #[serde(default)]
    pub consumers: Vec<Consumer>,
}

/// A queue consumer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Consumer {
    /// The queue.
    pub queue: String,
    /// The largest batch delivered at once.
    #[serde(default)]
    pub max_batch_size: Option<u32>,
}

/// A spec, and the unit it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The unit, e.g. `infra/pigeon`.
    pub unit: String,
    /// The spec's directory, from the root.
    pub dir: String,
    /// The spec.
    pub spec: Spec,
}

/// An item of a list output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// By position.
    At(usize),
    /// The one equal to this, or this, `-` and one more segment, as a suffixed name is.
    Named(String),
}

/// A `$output.<unit>.<name>[<item>]` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The unit.
    pub unit: String,
    /// The output.
    pub output: String,
    /// An item of a list output.
    pub index: Option<Item>,
}

/// Parses a `$output` reference.
#[must_use]
pub fn reference(s: &str) -> Option<Reference> {
    let rest = s.strip_prefix("$output.")?;
    let (unit, last) = rest.rsplit_once('.')?;
    let (output, index) = match last.split_once('[') {
        Some((o, i)) => {
            let i = i.strip_suffix(']')?;
            let item = match i.parse() {
                Ok(n) => Item::At(n),
                Err(_)
                    if !i.is_empty()
                        && i.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') =>
                {
                    Item::Named(i.to_string())
                }
                Err(_) => return None,
            };
            (o, Some(item))
        }
        None => (last, None),
    };
    let word = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    (unit.split('/').count() == 2 && unit.split('/').all(word) && word(output)).then(|| Reference {
        unit: unit.to_string(),
        output: output.to_string(),
        index,
    })
}

fn screaming(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// A class's binding name: `ChangeRun` is `CHANGE_RUN`.
#[must_use]
pub fn binding_of(class: &str) -> String {
    let mut out = String::new();
    for (i, c) in class.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

fn looks_like_id(s: &str) -> bool {
    s.len() >= 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Every spec under `<root>/infra/*/workers/*/`.
///
/// # Errors
///
/// [`Error::Invalid`] naming each spec that can't be read or parsed.
pub fn discover(root: &Path) -> Result<Vec<Found>, Error> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let Ok(units) = fs::read_dir(root.join("infra")) else {
        return Ok(found);
    };
    let mut units: Vec<_> = units.flatten().map(|e| e.path()).collect();
    units.sort();
    for unit in units {
        let Ok(workers) = fs::read_dir(unit.join("workers")) else {
            continue;
        };
        let mut dirs: Vec<_> = workers.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            let path = dir.join(SPEC);
            if !path.is_file() {
                continue;
            }
            let rel = |p: &Path| {
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            };
            match fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_saphyr::from_str::<Spec>(&t).map_err(|e| e.to_string()))
            {
                Ok(spec) => found.push(Found {
                    unit: rel(&unit),
                    dir: rel(&dir),
                    spec,
                }),
                Err(message) => problems.push(Problem {
                    path: rel(&path),
                    message,
                }),
            }
        }
    }
    if problems.is_empty() {
        Ok(found)
    } else {
        Err(Error::Invalid(problems))
    }
}

/// The specs named, or every spec.
///
/// # Errors
///
/// As [`discover`], and [`Error::UnknownUnit`] for an unknown name.
pub fn select(root: &Path, names: &[String]) -> Result<Vec<Found>, Error> {
    let all = discover(root)?;
    let aliases: Vec<Vec<String>> = all.iter().map(|f| known_as(root, f)).collect();
    for n in names {
        if !aliases.iter().any(|k| k.contains(n)) {
            return Err(Error::UnknownUnit(format!("workers/{n}")));
        }
    }
    Ok(all
        .into_iter()
        .zip(aliases)
        .filter(|(_, k)| names.is_empty() || k.iter().any(|n| names.contains(n)))
        .map(|(f, _)| f)
        .collect())
}

/// The names `found` answers to: its own, and for a stack-bound spec, the name each stack of
/// its kind gives it.
fn known_as(root: &Path, found: &Found) -> Vec<String> {
    let mut out = vec![found.spec.name.clone()];
    if let Some(kind) = &found.spec.deployment {
        out.extend(
            stacks(root, kind)
                .iter()
                .filter_map(|u| bind(root, found, u).ok())
                .map(|b| b.spec.name),
        );
    }
    out
}

/// The name `found` deploys as: bound as a deploy binds it with no stack named, else its own.
#[must_use]
pub fn deployed_name(root: &Path, found: &Found) -> String {
    bound(root, found, None).map_or_else(|_| found.spec.name.clone(), |b| b.spec.name)
}

/// As [`select`], each spec bound with [`bind`] to the stack `deployment`.
///
/// # Errors
///
/// As [`select`], and [`Error::Invalid`] for a spec that names a stack kind when `deployment`
/// is none, or that can't be bound to it.
pub fn select_bound(
    root: &Path,
    names: &[String],
    deployment: Option<&str>,
) -> Result<Vec<Found>, Error> {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for f in select(root, names)? {
        let path = format!("{}/{SPEC}", f.dir);
        match bound(root, &f, deployment) {
            Ok(b) => out.push(b),
            Err(message) => problems.push(Problem { path, message }),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(Error::Invalid(problems))
    }
}

/// Every problem with `found`'s spec, against the files under `root`.
#[must_use]
pub fn validate(root: &Path, found: &Found) -> Vec<Problem> {
    let file = format!("{}/{SPEC}", found.dir);
    let problem = |message: String| Problem {
        path: file.clone(),
        message,
    };
    if let Some(kind) = &found.spec.deployment {
        // A stack-bound spec is checked as each stack of its kind binds it.
        let units = stacks(root, kind);
        if units.is_empty() {
            return vec![problem(format!(
                "`deployment`: no unit of kind `{kind}` in config.yaml"
            ))];
        }
        let mut out = Vec::new();
        for u in units {
            match bind(root, found, &u) {
                Ok(bound) => out.extend(validate(root, &bound)),
                Err(e) => out.push(problem(format!("bound to `{u}`: {e}"))),
            }
        }
        return out;
    }
    let mut out = Vec::new();
    if serde_json::to_string(&found.spec).is_ok_and(|t| t.contains(DEPLOYMENT_PREFIX)) {
        out.push(problem(format!(
            "`{DEPLOYMENT_PREFIX}…` needs `deployment: <catalog>/<kind>`"
        )));
    }
    let mut add = |m: String| {
        out.push(Problem {
            path: file.clone(),
            message: m,
        });
    };
    let s = &found.spec;
    if s.name.is_empty()
        || s.name.len() > 63
        || !s
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        add(format!(
            "`name` `{}` must be lowercase letters, digits and `-`",
            s.name
        ));
    }
    let date = s.compatibility_date.as_bytes();
    if date.len() != 10 || date[4] != b'-' || date[7] != b'-' {
        add("`compatibility_date` must be YYYY-MM-DD".into());
    }
    if !root.join(&s.entry.krate).join("Cargo.toml").is_file() {
        add(format!(
            "`entry.crate` `{}` holds no Cargo.toml",
            s.entry.krate
        ));
    }
    validate_classes(s, root, &mut add);
    validate_bindings(s, root, &mut add);
    out
}

fn validate_classes(s: &Spec, root: &Path, add: &mut impl FnMut(String)) {
    let classes: BTreeSet<&str> = s
        .durable_objects
        .classes
        .iter()
        .map(String::as_str)
        .collect();
    for m in &s.durable_objects.migrations {
        for c in &m.new_sqlite_classes {
            if !classes.contains(c.as_str()) {
                add(format!("migration `{}` names unknown class `{c}`", m.tag));
            }
        }
    }
    for c in &s.containers {
        if !classes.contains(c.class.as_str()) {
            add(format!(
                "container `{}` must also be a Durable Object class",
                c.class
            ));
        }
        if !INSTANCE_TYPES.contains(&c.instance_type.as_str()) {
            add(format!(
                "container `{}`: instance type `{}` is not one of {}",
                c.class,
                c.instance_type,
                INSTANCE_TYPES.join(", ")
            ));
        }
        if !root.join(&c.image.dockerfile).is_file() {
            add(format!(
                "container `{}`: no Dockerfile `{}`",
                c.class, c.image.dockerfile
            ));
        }
    }
}

fn validate_bindings(s: &Spec, root: &Path, add: &mut impl FnMut(String)) {
    let b = &s.bindings;
    let mut names: Vec<&String> = b.d1.keys().collect();
    names.extend(b.r2.keys());
    names.extend(b.secrets_store.keys());
    names.extend(b.queues.producers.keys());
    names.extend(b.vars.keys());
    names.extend(&b.version_metadata);
    names.extend(&s.secrets);
    let mut seen = BTreeSet::new();
    for n in names {
        if !screaming(n) {
            add(format!("binding `{n}` must be SCREAMING_SNAKE_CASE"));
        }
        if !seen.insert(n.clone()) {
            add(format!("binding `{n}` is declared twice"));
        }
    }
    // Each Durable Object class is bound as its name in SCREAMING_SNAKE_CASE, beside the others.
    for c in &s.durable_objects.classes {
        let b = binding_of(c);
        if seen.contains(&b) {
            add(format!(
                "binding `{b}` is also Durable Object `{c}`'s binding; rename it"
            ));
        }
    }
    let mut refs: Vec<&String> = b.d1.values().map(|d| &d.id).collect();
    refs.extend(b.r2.values());
    refs.extend(b.secrets_store.values().map(|x| &x.store));
    for r in refs {
        match reference(r) {
            None => add(format!("`{r}` must be a `$output.<unit>.<name>` reference")),
            Some(x) if !root.join(&x.unit).is_dir() && !is_manifest_unit(root, &x.unit) => {
                add(format!("`{r}` names unknown unit `{}`", x.unit));
            }
            Some(_) => {}
        }
    }
    for d in b.d1.values() {
        if let Some(m) = &d.migrations
            && !root.join(m).is_dir()
        {
            add(format!("D1 `{}`: no migrations directory `{m}`", d.name));
        }
    }
    for (k, v) in &b.vars {
        if looks_like_id(v) {
            add(format!("var `{k}` looks like an ID; use a reference"));
        }
    }
}

fn is_manifest_unit(root: &Path, unit: &str) -> bool {
    crate::manifest::load(root, root).is_some_and(|(m, _)| m.units.contains_key(unit))
}

/// Every `$output` reference `spec` makes.
#[must_use]
pub fn references(spec: &Spec) -> Vec<String> {
    let b = &spec.bindings;
    b.d1.values()
        .map(|d| d.id.clone())
        .chain(b.r2.values().cloned())
        .chain(b.secrets_store.values().map(|x| x.store.clone()))
        .collect()
}

/// The units whose outputs `spec` references.
#[must_use]
pub fn referenced_units(spec: &Spec) -> BTreeSet<String> {
    references(spec)
        .iter()
        .filter_map(|r| reference(r))
        .map(|r| r.unit)
        .collect()
}

/// Whether `s` is `name`, or `name-<suffix>` with a suffix of one segment, as a bucket's
/// random suffix is.
fn named(s: &str, name: &str) -> bool {
    s == name
        || s.strip_prefix(name)
            .and_then(|t| t.strip_prefix('-'))
            .is_some_and(|t| !t.is_empty() && !t.contains('-'))
}

/// An output's shape for messages: its type and size, never a value.
fn shape(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("a list of {}", a.len()),
        Value::Object(o) => format!("a map of {}", o.len()),
        Value::String(_) => "a string".into(),
        Value::Null => "nothing".into(),
        _ => "a scalar".into(),
    }
}

/// Resolves a reference against `outputs`, each unit's outputs by name.
///
/// # Errors
///
/// A message naming the reference when its unit, output or index is missing, or its value is
/// not a string.
pub fn resolve(r: &str, outputs: &BTreeMap<String, Map<String, Value>>) -> Result<String, String> {
    let x = reference(r).ok_or_else(|| format!("`{r}` is not a reference"))?;
    let v = outputs
        .get(&x.unit)
        .and_then(|o| o.get(&x.output))
        .ok_or_else(|| format!("`{r}`: `{}` has no output `{}` yet", x.unit, x.output))?;
    let v = match x.index {
        Some(Item::At(i)) => v
            .as_array()
            .and_then(|a| a.get(i))
            .ok_or_else(|| format!("`{r}`: no item {i} in {}", shape(v)))?,
        // `$module.<block>[*].<out>` over a `for_each` renders a map, keyed by each item.
        Some(Item::Named(n)) if v.is_object() => {
            let Some(x) = v.get(&n) else {
                let keys: Vec<&str> = v
                    .as_object()
                    .into_iter()
                    .flat_map(|o| o.keys())
                    .map(String::as_str)
                    .collect();
                return Err(format!(
                    "`{r}`: no key `{n}`; its keys are {}",
                    keys.join(", ")
                ));
            };
            x
        }
        Some(Item::Named(n)) => {
            let found: Vec<&Value> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter(|x| x.as_str().is_some_and(|s| named(s, &n)))
                .collect();
            match found.as_slice() {
                [one] => *one,
                [] => {
                    return Err(format!("`{r}`: no item named `{n}` in {}", shape(v)));
                }
                _ => return Err(format!("`{r}`: more than one item named `{n}`")),
            }
        }
        None => v,
    };
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("`{r}` is not a string"))
}

/// The local tag deploy builds a container's image as.
#[must_use]
pub fn image_tag(spec: &Spec, class: &str, plan: &str) -> String {
    format!(
        "pigeon-local/{}-{}:{plan}",
        spec.name,
        class.to_ascii_lowercase()
    )
}

/// From the render directory up to the root.
const UP: &str = "../../..";

/// `found`'s `wrangler.jsonc`, with references resolved and each container's image built from
/// its stage under plan `plan`; paths are relative to [`RENDER_DIR`]`/<name>/`.
///
/// # Errors
///
/// A message for each reference that doesn't resolve.
pub fn render(
    found: &Found,
    outputs: &BTreeMap<String, Map<String, Value>>,
) -> Result<Value, Vec<String>> {
    render_with(found, outputs, None)
}

/// As [`render`], but with each container's `image` a registry reference from `images`, keyed
/// `<worker>/<class>`, when given: such a deploy needs no Docker; see
/// `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md` §4.
///
/// # Errors
///
/// A message for each reference that doesn't resolve, and each container `images` lacks.
pub fn render_with(
    found: &Found,
    outputs: &BTreeMap<String, Map<String, Value>>,
    images: Option<&BTreeMap<String, String>>,
) -> Result<Value, Vec<String>> {
    let spec = &found.spec;
    let mut problems = Vec::new();
    if let Some(images) = images {
        for c in &spec.containers {
            if !images.contains_key(&image_key(spec, &c.class)) {
                problems.push(format!(
                    "no image for `{}`; push one with `pigeon cobot workers images push`",
                    image_key(spec, &c.class)
                ));
            }
        }
    }
    let mut get = |r: &str| {
        resolve(r, outputs).unwrap_or_else(|e| {
            problems.push(e);
            String::new()
        })
    };
    let mut out = render_script(spec, images);
    render_bindings(&spec.bindings, &mut out, &mut get);
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// A container's key in an image map: `<worker>/<class>`.
#[must_use]
pub fn image_key(spec: &Spec, class: &str) -> String {
    format!("{}/{class}", spec.name)
}

/// The script, its triggers, classes and containers.
fn render_script(spec: &Spec, images: Option<&BTreeMap<String, String>>) -> Value {
    let features = spec.entry.features.join(",");
    let mut build = "worker-build --release --no-default-features".to_string();
    if !features.is_empty() {
        let _ = write!(build, " --features {features}");
    }
    let mut out = json!({
        "$schema": "node_modules/wrangler/config-schema.json",
        "name": spec.name,
        "main": format!("{UP}/{}/build/worker/shim.mjs", spec.entry.krate),
        "compatibility_date": spec.compatibility_date,
        "build": { "command": build, "cwd": format!("{UP}/{}", spec.entry.krate) },
        "observability": { "enabled": true },
        "workers_dev": spec.workers_dev,
    });
    if !spec.routes.is_empty() {
        out["routes"] = spec
            .routes
            .iter()
            .map(|r| json!({ "pattern": r.custom_domain, "custom_domain": true }))
            .collect();
    }
    if !spec.crons.is_empty() {
        out["triggers"] = json!({ "crons": spec.crons });
    }
    let objects = &spec.durable_objects;
    if !objects.classes.is_empty() {
        out["durable_objects"] = json!({
            "bindings": objects.classes.iter()
                .map(|c| json!({ "name": binding_of(c), "class_name": c }))
                .collect::<Vec<_>>(),
        });
        out["migrations"] = objects
            .migrations
            .iter()
            .map(|m| json!({ "tag": m.tag, "new_sqlite_classes": m.new_sqlite_classes }))
            .collect();
    }
    if !spec.containers.is_empty() {
        out["containers"] = spec
            .containers
            .iter()
            .map(|c| {
                json!({
                    "class_name": c.class,
                    "image": images
                        .and_then(|i| i.get(&image_key(spec, &c.class)))
                        .cloned()
                        .unwrap_or_else(|| format!("./{}.Dockerfile", c.class)),
                    "instance_type": c.instance_type,
                    "max_instances": c.max_instances,
                })
            })
            .collect();
    }
    out
}

/// D1, R2, Secrets Store, queues and vars, with references resolved through `get`.
/// The jurisdiction of every R2 bucket a Worker binds: the catalog's `cloudflare/r2-bucket`
/// default. Without it a binding names a default-jurisdiction bucket of the same name, which
/// wrangler creates empty; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md`.
pub const R2_JURISDICTION: &str = "eu";

fn render_bindings(bindings: &Bindings, out: &mut Value, get: &mut impl FnMut(&str) -> String) {
    if !bindings.d1.is_empty() {
        out["d1_databases"] = bindings
            .d1
            .iter()
            .map(|(k, d)| {
                let mut v =
                    json!({ "binding": k, "database_name": d.name, "database_id": get(&d.id) });
                if let Some(m) = &d.migrations {
                    v["migrations_dir"] = json!(format!("{UP}/{m}"));
                }
                v
            })
            .collect();
    }
    if !bindings.r2.is_empty() {
        out["r2_buckets"] = bindings
            .r2
            .iter()
            .map(|(k, r)| {
                json!({ "binding": k, "bucket_name": get(r), "jurisdiction": R2_JURISDICTION })
            })
            .collect();
    }
    if !bindings.secrets_store.is_empty() {
        out["secrets_store_secrets"] = bindings
            .secrets_store
            .iter()
            .map(|(k, x)| json!({ "binding": k, "store_id": get(&x.store), "secret_name": x.secret }))
            .collect();
    }
    let queues = &bindings.queues;
    if !queues.producers.is_empty() || !queues.consumers.is_empty() {
        let mut q = Map::new();
        if !queues.producers.is_empty() {
            q.insert(
                "producers".into(),
                queues
                    .producers
                    .iter()
                    .map(|(k, n)| json!({ "binding": k, "queue": n }))
                    .collect(),
            );
        }
        if !queues.consumers.is_empty() {
            q.insert(
                "consumers".into(),
                queues
                    .consumers
                    .iter()
                    .map(|c| {
                        let mut v = json!({ "queue": c.queue });
                        if let Some(n) = c.max_batch_size {
                            v["max_batch_size"] = json!(n);
                        }
                        v
                    })
                    .collect(),
            );
        }
        out["queues"] = Value::Object(q);
    }
    if !bindings.vars.is_empty() {
        out["vars"] = json!(bindings.vars);
    }
    if let Some(v) = &bindings.version_metadata {
        out["version_metadata"] = json!({ "binding": v });
    }
}

/// `value` as the rendered file's text: pretty JSON, which is valid JSONC, with a newline.
#[must_use]
pub fn text(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).unwrap_or_default();
    s.push('\n');
    s
}

/// The Dockerfile `render` writes for a container: its stage's image, built at deploy.
#[must_use]
pub fn container_dockerfile(spec: &Spec, class: &str, plan: &str) -> String {
    format!(
        "# Built by `pigeon cobot workers deploy` from its spec's stage; never committed.\nFROM --platform={PLATFORM} {}\n",
        image_tag(spec, class, plan)
    )
}

/// Writes `found`'s rendered files to `<root>/`[`RENDER_DIR`]`/<name>/`; returns that
/// directory.
///
/// # Errors
///
/// [`Error::Io`] if a file can't be written.
pub fn write(root: &Path, found: &Found, rendered: &Value, plan: &str) -> Result<PathBuf, Error> {
    let dir = root.join(RENDER_DIR).join(&found.spec.name);
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |e: std::io::Error| Error::Io {
            path: p,
            message: e.to_string(),
        }
    };
    fs::create_dir_all(&dir).map_err(io(&dir))?;
    let file = dir.join("wrangler.jsonc");
    fs::write(&file, text(rendered)).map_err(io(&file))?;
    for c in &found.spec.containers {
        let f = dir.join(format!("{}.Dockerfile", c.class));
        fs::write(&f, container_dockerfile(&found.spec, &c.class, plan)).map_err(io(&f))?;
    }
    Ok(dir)
}

/// The token permissions deploying `specs` needs, on the account.
#[must_use]
pub fn permissions(specs: &[Spec]) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = [
        "Workers Scripts Write (Workers Admin)",
        "Account Settings Read",
        "Workers KV Storage Read",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    let mut add = |p: &str| {
        out.insert(p.to_string());
    };
    for s in specs {
        if !s.containers.is_empty() {
            add("Workers Containers Read and Write");
        }
        if !s.bindings.d1.is_empty() {
            add("D1 Read and Write");
        }
        if !s.bindings.r2.is_empty() {
            add("Workers R2 Storage Read and Write");
        }
        if !s.bindings.secrets_store.is_empty() {
            add("Secrets Store Read and Write");
        }
        if !s.bindings.queues.producers.is_empty() || !s.bindings.queues.consumers.is_empty() {
            add("Queues Read and Write");
        }
        if !s.routes.is_empty() {
            add("SSL and Certificates Read and Write");
            add("Workers Routes Write (zone)");
        }
    }
    out
}

/// A stored Worker plan: what deploy will do, valid for [`crate::plans::LIFETIME_SECS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// Its ID.
    pub id: String,
    /// When it was made, in Unix seconds.
    pub created: u64,
    /// When it expires.
    pub expires: u64,
    /// Each Worker's name and its rendered configuration's digest.
    pub workers: BTreeMap<String, String>,
}

/// The digest of a rendered configuration.
#[must_use]
pub fn digest(rendered: &Value) -> String {
    hex(&Sha256::digest(text(rendered).as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Stores a plan for `rendered`, each Worker's name and configuration, in `state`.
///
/// # Errors
///
/// [`Error::Io`] if it can't be written.
pub fn store(state: &Path, rendered: &BTreeMap<String, Value>, now: u64) -> Result<Stored, Error> {
    let workers: BTreeMap<String, String> = rendered
        .iter()
        .map(|(n, v)| (n.clone(), digest(v)))
        .collect();
    let mut h = Sha256::new();
    h.update(now.to_be_bytes());
    for (n, d) in &workers {
        h.update(n.as_bytes());
        h.update(d.as_bytes());
    }
    let stored = Stored {
        id: hex(&h.finalize()[..16]),
        created: now,
        expires: now + crate::plans::LIFETIME_SECS,
        workers,
    };
    let file = state.join(PLANS_DIR).join(format!("{}.json", stored.id));
    let io = |e: std::io::Error| Error::Io {
        path: file.clone(),
        message: e.to_string(),
    };
    fs::create_dir_all(state.join(PLANS_DIR)).map_err(io)?;
    fs::write(&file, text(&json!(stored))).map_err(io)?;
    Ok(stored)
}

/// Loads plan `id`, refusing one that is unknown or expired.
///
/// # Errors
///
/// [`Error::Plan`].
pub fn load(state: &Path, id: &str, now: u64) -> Result<Stored, Error> {
    if !valid_id(id) {
        return Err(Error::Plan(format!("`{id}` is not a plan ID")));
    }
    let file = state.join(PLANS_DIR).join(format!("{id}.json"));
    let stored: Stored = fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| Error::Plan(format!("no stored Worker plan {id}")))?;
    if stored.id != id {
        return Err(Error::Plan(format!("plan {id}'s file does not match it")));
    }
    if now >= stored.expires {
        return Err(Error::Plan(format!("plan {id} expired; plan again")));
    }
    Ok(stored)
}

/// `text` with `secrets` and any 32-or-more-character hex run, an account or resource ID, cut
/// to its last four characters.
#[must_use]
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut s = text.to_string();
    for x in secrets.iter().filter(|x| x.len() >= 8) {
        s = s.replace(x, "…");
    }
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 32 {
            out.push('…');
            out.push_str(&run[run.len() - 4..]);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// wrangler and docker, and the environment and secrets they run with.
#[derive(Clone)]
pub struct Tools {
    /// wrangler's path or name.
    pub wrangler: PathBuf,
    /// docker's path or name.
    pub docker: PathBuf,
    /// Extra environment, e.g. `CLOUDFLARE_ACCOUNT_ID`; never printed.
    pub env: Vec<(String, String)>,
    /// Values scrubbed from every output.
    pub secrets: Vec<String>,
}

impl std::fmt::Debug for Tools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tools")
            .field("wrangler", &self.wrangler)
            .field("docker", &self.docker)
            .field("env", &self.env.iter().map(|(k, _)| k).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Tools {
    /// From `var`: [`WRANGLER_VAR`] and [`DOCKER_VAR`], else `wrangler` and `docker`.
    pub fn from(var: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            wrangler: var(WRANGLER_VAR).map_or_else(|| "wrangler".into(), PathBuf::from),
            docker: var(DOCKER_VAR).map_or_else(|| "docker".into(), PathBuf::from),
            env: Vec::new(),
            secrets: Vec::new(),
        }
    }

    fn scrubbed(&self, t: &str) -> String {
        let s: Vec<&str> = self
            .secrets
            .iter()
            .map(String::as_str)
            .chain(self.env.iter().map(|(_, v)| v.as_str()))
            .collect();
        scrub(t, &s)
    }

    /// Runs `program args` in `dir`, with `input` on stdin; its scrubbed output.
    ///
    /// # Errors
    ///
    /// [`Error::Tool`] when it can't start or fails, with its scrubbed output.
    pub fn run(
        &self,
        program: &Path,
        args: &[String],
        dir: &Path,
        input: Option<&str>,
    ) -> Result<String, Error> {
        let shown = format!("{} {}", program.display(), args.join(" "));
        let mut child = Command::new(program)
            .args(args)
            .current_dir(dir)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Tool {
                command: self.scrubbed(&shown),
                message: e.to_string(),
            })?;
        if let (Some(i), Some(mut stdin)) = (input, child.stdin.take()) {
            let _ = stdin.write_all(i.as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| Error::Tool {
            command: self.scrubbed(&shown),
            message: e.to_string(),
        })?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() {
            Ok(self.scrubbed(&text))
        } else {
            Err(Error::Tool {
                command: self.scrubbed(&shown),
                message: self.scrubbed(text.trim()),
            })
        }
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

/// Deploys `found` from its rendered directory `dir` under plan `plan`, in order: D1
/// migrations, each container's image, then the Worker; returns each step's name.
///
/// # Errors
///
/// [`Error::Tool`] from the first step that fails; later steps don't run.
pub fn deploy(
    root: &Path,
    found: &Found,
    dir: &Path,
    plan: &str,
    tools: &Tools,
) -> Result<Vec<String>, Error> {
    let mut done = migrate(found, dir, tools)?;
    done.extend(build_images(
        root,
        found,
        &|class| image_tag(&found.spec, class, plan),
        tools,
    )?);
    done.push(wrangler_deploy(found, dir, tools)?);
    Ok(done)
}

/// Applies `found`'s D1 migrations from its rendered directory `dir`; each step's name.
///
/// # Errors
///
/// [`Error::Tool`] when wrangler fails.
pub fn migrate(found: &Found, dir: &Path, tools: &Tools) -> Result<Vec<String>, Error> {
    let mut done = Vec::new();
    for d in found.spec.bindings.d1.values() {
        if d.migrations.is_some() {
            tools.run(
                &tools.wrangler,
                &strings(&[
                    "d1",
                    "migrations",
                    "apply",
                    &d.name,
                    "--remote",
                    "-c",
                    "wrangler.jsonc",
                ]),
                dir,
                None,
            )?;
            done.push(format!("D1 {} migrated", d.name));
        }
    }
    Ok(done)
}

/// Builds each of `found`'s container images from its Dockerfile stage with Docker, tagged
/// `tag(class)`; each step's name.
///
/// # Errors
///
/// [`Error::Tool`] when Docker fails.
pub fn build_images(
    root: &Path,
    found: &Found,
    tag: &dyn Fn(&str) -> String,
    tools: &Tools,
) -> Result<Vec<String>, Error> {
    let mut done = Vec::new();
    for c in &found.spec.containers {
        let file = root.join(&c.image.dockerfile);
        let context = file.parent().unwrap_or(root);
        let mut args: Vec<String> = vec![
            "build".into(),
            "--platform".into(),
            PLATFORM.into(),
            "--target".into(),
            c.image.target.clone(),
        ];
        for (k, v) in &c.image.args {
            args.extend(["--build-arg".into(), format!("{k}={v}")]);
        }
        args.extend([
            "-t".into(),
            tag(&c.class),
            "-f".into(),
            file.to_string_lossy().into_owned(),
            context.to_string_lossy().into_owned(),
        ]);
        tools.run(&tools.docker, &args, root, None)?;
        done.push(format!("image {} built from {}", c.class, c.image.target));
    }
    Ok(done)
}

/// `wrangler deploy` of `found` from its rendered directory `dir`; the step's name.
///
/// # Errors
///
/// [`Error::Tool`] when wrangler fails.
pub fn wrangler_deploy(found: &Found, dir: &Path, tools: &Tools) -> Result<String, Error> {
    tools.run(
        &tools.wrangler,
        &strings(&["deploy", "-c", "wrangler.jsonc"]),
        dir,
        None,
    )?;
    Ok(format!("{} deployed", found.spec.name))
}

/// Cloudflare's container registry.
pub const REGISTRY: &str = "registry.cloudflare.com";

/// How long `images push` logs in to the registry for, by default: longer than a slow upload
/// of a large image; see `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §1.
pub const LOGIN_MINUTES: u32 = 240;

/// Registry credentials with push and pull, valid `minutes`, minted with `api`'s token: the
/// username and password `docker login` takes.
///
/// # Errors
///
/// A message naming the call that failed, never the password.
pub fn registry_credentials(
    api: &crate::cloudflare::Api,
    account: &str,
    minutes: u32,
) -> Result<(String, String), String> {
    let v = api.post(
        &format!("/accounts/{account}/containers/registries/{REGISTRY}/credentials"),
        &json!({ "expiration_minutes": minutes, "permissions": ["push", "pull"] }),
    )?;
    match (v["username"].as_str(), v["password"].as_str()) {
        (Some(u), Some(p)) if !p.is_empty() => Ok((u.to_string(), p.to_string())),
        _ => Err("the registry's credentials answer has no username and password".into()),
    }
}

/// The waits before each retry of a failed `docker push`; a retry skips the layers already
/// pushed.
pub const PUSH_WAITS: [std::time::Duration; 2] = [
    std::time::Duration::from_secs(10),
    std::time::Duration::from_secs(30),
];

/// Builds each of `found`'s container images, logs Docker in to Cloudflare's registry for
/// `minutes`, and pushes each as `<worker>-<class>:<tag>`, retrying a failed push after each of
/// [`PUSH_WAITS`]; each image's registry reference, keyed as [`image_key`]. The password reaches
/// Docker on stdin only and is scrubbed from every output; Docker is logged out after, whatever
/// happened.
///
/// # Errors
///
/// [`Error::Tool`] when Docker or the registry's API fails.
pub fn push_images(
    root: &Path,
    found: &Found,
    tag: &str,
    account: &str,
    api: &crate::cloudflare::Api,
    minutes: u32,
    tools: &Tools,
) -> Result<BTreeMap<String, String>, Error> {
    push_images_waiting(root, found, tag, account, api, minutes, tools, &PUSH_WAITS)
}

/// [`push_images`], waiting `waits` before each retry of a failed push.
///
/// # Errors
///
/// As [`push_images`].
#[allow(clippy::too_many_arguments)]
pub fn push_images_waiting(
    root: &Path,
    found: &Found,
    tag: &str,
    account: &str,
    api: &crate::cloudflare::Api,
    minutes: u32,
    tools: &Tools,
    waits: &[std::time::Duration],
) -> Result<BTreeMap<String, String>, Error> {
    let local = |class: &str| format!("{}-{}:{tag}", found.spec.name, class.to_ascii_lowercase());
    build_images(root, found, &local, tools)?;
    let (user, password) =
        registry_credentials(api, account, minutes).map_err(|message| Error::Tool {
            command: "registry credentials".into(),
            message,
        })?;
    let mut tools = tools.clone();
    tools.secrets.push(password.clone());
    tools.run(
        &tools.docker,
        &strings(&["login", REGISTRY, "--username", &user, "--password-stdin"]),
        root,
        Some(&password),
    )?;
    let pushed = (|| {
        let mut out = BTreeMap::new();
        for c in &found.spec.containers {
            let remote = format!("{REGISTRY}/{account}/{}", local(&c.class));
            tools.run(
                &tools.docker,
                &strings(&["tag", &local(&c.class), &remote]),
                root,
                None,
            )?;
            let mut waits = waits.iter();
            loop {
                match tools.run(&tools.docker, &strings(&["push", &remote]), root, None) {
                    Ok(_) => break,
                    Err(e) => {
                        let Some(wait) = waits.next() else {
                            return Err(e);
                        };
                        eprintln!(
                            "cobot: the push of {} failed; retrying in {}s: {e}",
                            local(&c.class),
                            wait.as_secs()
                        );
                        std::thread::sleep(*wait);
                    }
                }
            }
            out.insert(image_key(&found.spec, &c.class), remote);
        }
        Ok::<_, Error>(out)
    })();
    let _ = tools.run(&tools.docker, &strings(&["logout", REGISTRY]), root, None);
    pushed
}

/// The spec's secrets that the deployed Worker lacks.
///
/// # Errors
///
/// [`Error::Tool`] when wrangler fails.
pub fn missing_secrets(found: &Found, tools: &Tools, dir: &Path) -> Result<Vec<String>, Error> {
    if found.spec.secrets.is_empty() {
        return Ok(Vec::new());
    }
    let listed = tools.run(
        &tools.wrangler,
        &strings(&[
            "secret",
            "list",
            "--name",
            &found.spec.name,
            "--format",
            "json",
        ]),
        dir,
        None,
    )?;
    let have: BTreeSet<String> = serde_json::from_str::<Value>(&listed)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["name"].as_str().map(str::to_string))
        .collect();
    Ok(found
        .spec
        .secrets
        .iter()
        .filter(|s| !have.contains(*s))
        .cloned()
        .collect())
}

/// Sets Worker secret `key` of `found` to `value`, through stdin.
///
/// # Errors
///
/// [`Error::Invalid`] for a key the spec doesn't declare; [`Error::Tool`] when wrangler fails.
pub fn put_secret(
    found: &Found,
    key: &str,
    value: &str,
    tools: &Tools,
    dir: &Path,
) -> Result<(), Error> {
    if !found.spec.secrets.iter().any(|s| s == key) {
        return Err(Error::Invalid(vec![Problem {
            path: format!("{}/{SPEC}", found.dir),
            message: format!("declares no secret `{key}`"),
        }]));
    }
    let mut t = tools.clone();
    t.secrets.push(value.to_string());
    t.run(
        &t.wrangler,
        &strings(&["secret", "put", key, "--name", &found.spec.name]),
        dir,
        Some(value),
    )
    .map(|_| ())
}

/// The deployed Worker's status, scrubbed.
///
/// # Errors
///
/// [`Error::Tool`] when wrangler fails.
pub fn status(found: &Found, tools: &Tools, dir: &Path) -> Result<String, Error> {
    tools.run(
        &tools.wrangler,
        &strings(&["deployments", "status", "--name", &found.spec.name]),
        dir,
        None,
    )
}

/// Deletes the Worker, then every Container application it owns.
///
/// # Errors
///
/// [`Error::Tool`] when wrangler fails.
pub fn teardown(found: &Found, tools: &Tools, dir: &Path) -> Result<Vec<String>, Error> {
    let name = &found.spec.name;
    let mut done = Vec::new();
    tools.run(
        &tools.wrangler,
        &strings(&["delete", "--name", name, "--force"]),
        dir,
        None,
    )?;
    done.push(format!("{name} deleted"));
    if found.spec.containers.is_empty() {
        return Ok(done);
    }
    // The raw listing holds IDs, which the command needs; it is never printed.
    let raw = Tools {
        secrets: Vec::new(),
        env: tools.env.clone(),
        ..tools.clone()
    };
    let listed = Command::new(&raw.wrangler)
        .args(["containers", "list", "--json"])
        .current_dir(dir)
        .envs(raw.env.iter().map(|(k, v)| (k, v)))
        .output()
        .map_err(|e| Error::Tool {
            command: "wrangler containers list".into(),
            message: e.to_string(),
        })?;
    let apps: Vec<Value> = serde_json::from_slice::<Value>(&listed.stdout)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    for app in apps {
        let (Some(id), Some(app_name)) = (app["id"].as_str(), app["name"].as_str()) else {
            continue;
        };
        if app_name == name || app_name.starts_with(&format!("{name}-")) {
            tools.run(
                &tools.wrangler,
                &strings(&["containers", "delete", id]),
                dir,
                Some("y\n"),
            )?;
            done.push(format!("container application {app_name} deleted"));
        }
    }
    Ok(done)
}
