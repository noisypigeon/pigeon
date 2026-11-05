//! Zone apexes: layout, presets, records, claims and the token handoff; see
//! `.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::generations::{Day, Generation};

/// A name no workload may take.
pub const APEX: &str = "apex";
/// The unit that mints apex tokens.
pub const CLOUDFLARE: &str = "infra/cloudflare";
/// The only permission an apex token holds.
pub const DNS_WRITE: &str = "DNS Write";
/// The only handoff kind.
pub const HANDOFF: &str = "zone-token";
/// The only derived-list expander.
pub const EXPANDER: &str = "zone-records";
/// An apex's handoff keys.
pub const HANDOFF_KEYS: [&str; 2] = ["api_token", "zone_id"];
/// Inert values a plan uses while the handoff is pending.
pub const PLACEHOLDER_TOKEN: &str = "0000000000000000000000000000000000000000";
/// See [`PLACEHOLDER_TOKEN`].
pub const PLACEHOLDER_ZONE_ID: &str = "00000000000000000000000000000000";

const RECORD_TYPES: [&str; 5] = ["A", "AAAA", "CNAME", "MX", "TXT"];
const ADDRESS_TYPES: [&str; 3] = ["A", "AAAA", "CNAME"];

static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid regex"));
static ZONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]+(\.[a-z0-9-]+)+$").expect("valid regex"));
static PLACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([^}]*)\}").expect("valid regex"));
static HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(@|(\*\.)?[a-z0-9_]([a-z0-9_-]*[a-z0-9_])?(\.[a-z0-9_]([a-z0-9_-]*[a-z0-9_])?)*)$",
    )
    .expect("valid regex")
});
static SECRET_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_]+$").expect("valid regex"));
static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_]+(\.[a-z0-9_]+)*$").expect("valid regex"));

/// Where an apex or a schema-less workload sits: the repository root, its
/// zone, and the workload's name for a workload.
#[derive(Debug, Clone)]
pub struct Site {
    /// The repository root.
    pub root: PathBuf,
    /// The zone of `workloads/<zone>/…`.
    pub zone: String,
    /// The workload of `workloads/<zone>/<workload>`; `None` for the apex.
    pub workload: Option<String>,
}

/// Whether `name` is shaped like a zone.
#[must_use]
pub fn is_zone(name: &str) -> bool {
    ZONE.is_match(name)
}

/// Whether `name` is a lowercase-dash workload name.
#[must_use]
pub fn is_name(name: &str) -> bool {
    NAME.is_match(name)
}

/// The zone of an apex path, `workloads/<zone>/apex`.
#[must_use]
pub fn zone_apex(path: &str) -> Option<&str> {
    let mut parts = path.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("workloads"), Some(zone), Some(APEX), None) if ZONE.is_match(zone) => Some(zone),
        _ => None,
    }
}

/// The zones an apex config names, in order.
#[must_use]
pub fn zones(config: &Value) -> Vec<String> {
    config["zones"]
        .as_object()
        .map(|z| z.keys().cloned().collect())
        .unwrap_or_default()
}

/// The token `infra/cloudflare` mints for a zone's apex.
#[must_use]
pub fn token_name(zone: &str) -> String {
    format!("apex-{zone}")
}

/// Escapes a literal for an HCL template.
fn escape(text: &str) -> String {
    text.replace("${", "$${").replace("%{", "%%{")
}

fn secret_ref(key: &str) -> String {
    format!("${{var.secrets[\"{key}\"]}}")
}

/// `{ secret: <key> }`'s key, when it is a plain key.
fn as_secret(value: &Value) -> Option<&str> {
    let o = value.as_object()?;
    (o.len() == 1)
        .then(|| o.get("secret")?.as_str())?
        .filter(|k| SECRET_KEY.is_match(k))
}

/// `{ secret: … }` values whose key is not a plain key.
fn bad_secrets(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(o) if o.contains_key("secret") && as_secret(v).is_none() => {
            out.push(format!("{at}: a secret is `{{ secret: <key> }}`, its key lowercase letters, digits and underscores"));
        }
        Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| bad_secrets(x, &format!("{at}.{k}"), out)),
        Value::Array(a) => a.iter().for_each(|x| bad_secrets(x, at, out)),
        _ => {}
    }
}

/// Every `{ secret: <key> }` key in `config`.
#[must_use]
pub fn referenced_secrets(config: &Value) -> BTreeSet<String> {
    fn walk(v: &Value, out: &mut BTreeSet<String>) {
        if let Some(k) = as_secret(v) {
            out.insert(k.to_string());
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(config, &mut out);
    out
}

/// A preset file, `infra/cloudflare/presets/<service>.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// One line saying what it is.
    pub description: String,
    /// Inputs by dotted path.
    #[serde(default)]
    pub inputs: BTreeMap<String, InputSpec>,
    /// What it owns.
    #[serde(default)]
    pub claims: Vec<ClaimSpec>,
    /// Its records by short key.
    pub records: BTreeMap<String, RecordSpec>,
}

/// One preset input.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    /// Its type.
    #[serde(rename = "type")]
    pub kind: InputType,
    /// Whether it may be `{ secret: <key> }`.
    #[serde(default)]
    pub secret: bool,
    /// Whether it may be unset.
    #[serde(default)]
    pub optional: bool,
    /// The value when absent or `default`.
    pub default: Option<Value>,
}

/// A preset input's type.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    /// A string.
    String,
    /// An integer.
    Integer,
    /// A boolean.
    Boolean,
}

/// What a claim reserves at a name.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClaimType {
    /// `A`, `AAAA` or `CNAME`.
    #[serde(rename = "ADDRESS")]
    Address,
    /// `MX`.
    #[serde(rename = "MX")]
    Mx,
    /// A `v=spf1` `TXT`.
    #[serde(rename = "SPF")]
    Spf,
    /// A `v=DMARC1` `TXT`.
    #[serde(rename = "DMARC")]
    Dmarc,
}

impl ClaimType {
    fn word(self) -> &'static str {
        match self {
            Self::Address => "ADDRESS",
            Self::Mx => "MX",
            Self::Spf => "SPF",
            Self::Dmarc => "DMARC",
        }
    }
}

/// One declared claim.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSpec {
    /// A name relative to the zone.
    pub name: String,
    /// What it reserves.
    #[serde(rename = "type")]
    pub kind: ClaimType,
    /// Held only while this input is set.
    pub when: Option<String>,
}

/// One preset record.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordSpec {
    /// A name relative to the zone.
    pub name: String,
    /// Its DNS type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its content, with `${input}` placeholders.
    pub content: String,
    /// Its TTL.
    pub ttl: Option<i64>,
    /// Its priority.
    pub priority: Option<i64>,
    /// Whether Cloudflare proxies it.
    pub proxied: Option<bool>,
}

/// A record being assembled: content is already an HCL template.
#[derive(Debug, Clone)]
struct Rec {
    owner: String,
    name: String,
    kind: String,
    content: String,
    /// The content as published, when known: literals, plus secrets once decrypted.
    raw: Option<String>,
    ttl: i64,
    priority: Option<i64>,
    proxied: Option<bool>,
}

/// A resolved input value.
#[derive(Debug, Clone)]
enum Resolved {
    Literal(String),
    Secret(String),
}

impl Resolved {
    fn template(&self) -> String {
        match self {
            Self::Literal(s) => escape(s),
            Self::Secret(k) => secret_ref(k),
        }
    }
}

/// Loads a preset.
///
/// # Errors
///
/// A message if the file is missing or invalid.
pub fn load_preset(home: &Path, service: &str) -> Result<Preset, String> {
    let path = home.join("presets").join(format!("{service}.yaml"));
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("service `{service}` has no preset at presets/{service}.yaml"))?;
    serde_saphyr::from_str(&text).map_err(|e| format!("presets/{service}.yaml: {e}"))
}

/// Checks a preset on its own; problems are messages.
#[must_use]
pub fn check_preset(service: &str, p: &Preset) -> Vec<String> {
    let mut out = Vec::new();
    for (path, spec) in &p.inputs {
        if !INPUT.is_match(path) {
            out.push(format!(
                "preset `{service}`: input `{path}` is not a dotted name"
            ));
        }
        if let Some(d) = &spec.default
            && !type_ok(spec.kind, d)
        {
            out.push(format!(
                "preset `{service}`: input `{path}`'s default has the wrong type"
            ));
        }
    }
    for c in &p.claims {
        if let Some(w) = &c.when
            && !p.inputs.contains_key(w)
        {
            out.push(format!(
                "preset `{service}`: claim names unknown input `{w}`"
            ));
        }
    }
    for (key, r) in &p.records {
        if !NAME.is_match(key) {
            out.push(format!(
                "preset `{service}`: record key `{key}` is not kebab-case"
            ));
        }
        for c in PLACE.captures_iter(&r.content) {
            if !p.inputs.contains_key(&c[1]) {
                out.push(format!(
                    "preset `{service}`: record `{key}` names unknown input `{}`",
                    &c[1]
                ));
            }
        }
    }
    out
}

fn type_ok(kind: InputType, v: &Value) -> bool {
    match kind {
        InputType::String => v.is_string(),
        InputType::Integer => v.is_i64(),
        InputType::Boolean => v.is_boolean(),
    }
}

fn literal_of(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// Flattens a service's config into dotted input paths.
fn flatten(
    service: &str,
    p: &Preset,
    value: &Value,
    prefix: &str,
    out: &mut BTreeMap<String, Value>,
    problems: &mut Vec<String>,
) {
    let Some(o) = value.as_object() else {
        problems.push(format!("services.{service}: must be a map of inputs"));
        return;
    };
    for (k, v) in o {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        if p.inputs.contains_key(&path) {
            out.insert(path, v.clone());
        } else if v.is_object()
            && as_secret(v).is_none()
            && p.inputs.keys().any(|i| i.starts_with(&format!("{path}.")))
        {
            flatten(service, p, v, &path, out, problems);
        } else {
            problems.push(format!("services.{service}: unknown input `{path}`"));
        }
    }
}

/// Resolves a service's inputs; unset inputs are absent.
fn resolve(
    service: &str,
    p: &Preset,
    config: &Value,
    problems: &mut Vec<String>,
) -> BTreeMap<String, Resolved> {
    let mut given = BTreeMap::new();
    flatten(service, p, config, "", &mut given, problems);
    let mut out = BTreeMap::new();
    for (path, spec) in &p.inputs {
        let at = format!("services.{service}.{path}");
        let value = match given.get(path) {
            Some(Value::String(s)) if s == "default" && spec.default.is_some() => {
                spec.default.clone()
            }
            Some(Value::Bool(false)) if spec.optional => None,
            Some(v) => Some(v.clone()),
            None => spec.default.clone(),
        };
        match value {
            None if spec.optional => {}
            None => problems.push(format!("{at}: is required")),
            Some(v) => {
                if let Some(k) = as_secret(&v) {
                    if spec.secret {
                        out.insert(path.clone(), Resolved::Secret(k.to_string()));
                    } else {
                        problems.push(format!("{at}: may not come from a secret"));
                    }
                } else if type_ok(spec.kind, &v) {
                    out.insert(path.clone(), Resolved::Literal(literal_of(&v)));
                } else {
                    problems.push(format!("{at}: must be a {:?}", spec.kind).to_lowercase());
                }
            }
        }
    }
    out
}

/// A preset template as published; `None` when a secret is not decrypted.
fn fill_raw(
    template: &str,
    values: &BTreeMap<String, Resolved>,
    secrets: &BTreeMap<String, String>,
) -> Option<String> {
    let mut out = String::new();
    let mut last = 0;
    for c in PLACE.captures_iter(template) {
        let m = c.get(0).expect("match");
        out.push_str(&template[last..m.start()]);
        match values.get(&c[1])? {
            Resolved::Literal(l) => out.push_str(l),
            Resolved::Secret(k) => out.push_str(secrets.get(k)?),
        }
        last = m.end();
    }
    out.push_str(&template[last..]);
    Some(out)
}

/// Renders a preset template; `None` when it names an unset input.
fn fill(template: &str, values: &BTreeMap<String, Resolved>) -> Option<String> {
    let mut out = String::new();
    let mut last = 0;
    for c in PLACE.captures_iter(template) {
        let m = c.get(0).expect("match");
        out.push_str(&escape(&template[last..m.start()]));
        out.push_str(&values.get(&c[1])?.template());
        last = m.end();
    }
    out.push_str(&escape(&template[last..]));
    Some(out)
}

/// One owner's claim.
type Claim = (String, ClaimType, String);

/// What `content` claims, for an explicit record of `kind`.
fn implied_claim(kind: &str, content: Option<&str>) -> Option<ClaimType> {
    if ADDRESS_TYPES.contains(&kind) {
        return Some(ClaimType::Address);
    }
    match (kind, content) {
        ("MX", _) => Some(ClaimType::Mx),
        ("TXT", Some(c)) if c.starts_with("v=spf1") => Some(ClaimType::Spf),
        ("TXT", Some(c)) if c.starts_with("v=DMARC1") => Some(ClaimType::Dmarc),
        _ => None,
    }
}

/// A host's key part: `apex` for `@`, dashes for dots.
pub(crate) fn label(host: &str) -> String {
    if host == "@" {
        "apex".into()
    } else {
        host.replace(['.', '_', '*'], "-")
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }
}

/// The workload units in a zone, by name; `true` for config units.
#[must_use]
pub fn workloads(root: &Path, zone: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let Ok(listing) = fs::read_dir(root.join("workloads").join(zone)) else {
        return out;
    };
    for e in listing.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let dir = e.path();
        if name == APEX || !dir.is_dir() {
            continue;
        }
        if dir.join(crate::config::CONFIG).is_file() {
            out.insert(name, true);
        } else if dir.join("unit.yaml").is_file() {
            out.insert(name, false);
        }
    }
    out
}

/// A workload's `config.yaml`, if it parses.
pub(crate) fn workload_config(root: &Path, zone: &str, workload: &str) -> Option<Value> {
    let text = fs::read_to_string(
        root.join("workloads")
            .join(zone)
            .join(workload)
            .join(crate::config::CONFIG),
    )
    .ok()?;
    serde_saphyr::from_str(&text).ok()
}

/// A workload's `ingress`, if its `config.yaml` has one.
fn ingress(root: &Path, zone: &str, workload: &str) -> Option<Value> {
    workload_config(root, zone, workload)?
        .get("ingress")
        .cloned()
}

/// Each workload's hosts in one zone's apex config: the apex workload at `@`
/// and `www`, every other at its name, unless `hosts` replaces them. Hosts
/// that are not relative host names are left out.
pub(crate) fn assignments(
    config: &Value,
    units: &BTreeMap<String, bool>,
    zone: &str,
) -> BTreeMap<String, Vec<String>> {
    let mut assigned: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(a) = config["apex"].as_str() {
        assigned.insert(a.to_string(), vec!["@".into(), "www".into()]);
    }
    for w in units.keys() {
        assigned.entry(w.clone()).or_insert_with(|| vec![w.clone()]);
    }
    let overrides = config["hosts"].as_object().cloned().unwrap_or_default();
    for (w, hosts) in &mut assigned {
        if let Some(list) = overrides.get(w).and_then(Value::as_array) {
            *hosts = list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
        }
        hosts.retain(|h| HOST.is_match(h) && h != zone && !h.ends_with(&format!(".{zone}")));
    }
    assigned
}

fn check_ingress(v: &Value) -> Result<(String, String, bool), String> {
    let o = v.as_object().ok_or("must be a map")?;
    if let Some(k) = o
        .keys()
        .find(|k| !["type", "content", "proxied"].contains(&k.as_str()))
    {
        return Err(format!("unknown key `{k}`"));
    }
    let kind = o.get("type").and_then(Value::as_str).unwrap_or_default();
    if !ADDRESS_TYPES.contains(&kind) {
        return Err("`type` must be A, AAAA or CNAME".into());
    }
    let content = o
        .get("content")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .ok_or("needs a non-empty `content`")?;
    let proxied = match o.get("proxied") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`proxied` must be a boolean".into()),
    };
    Ok((kind.to_string(), content.to_string(), proxied))
}

/// An apex's records and claims as they are assembled.
#[derive(Default)]
struct Expansion {
    /// Decrypted secrets, when planning; empty when validating.
    secrets: BTreeMap<String, String>,
    recs: BTreeMap<String, Rec>,
    claims: Vec<Claim>,
    prefixes: Vec<String>,
    problems: Vec<String>,
}

impl Expansion {
    /// Each enabled preset's records and claims.
    fn services(&mut self, config: &Value, home: &Path) {
        for (service, cfg) in config["services"].as_object().into_iter().flatten() {
            self.prefixes.push(format!("{service}-"));
            let preset = match load_preset(home, service) {
                Ok(p) => p,
                Err(e) => {
                    self.problems.push(format!("services.{service}: {e}"));
                    continue;
                }
            };
            self.problems.extend(check_preset(service, &preset));
            let values = resolve(service, &preset, cfg, &mut self.problems);
            for c in &preset.claims {
                if c.when.as_ref().is_none_or(|w| values.contains_key(w)) {
                    self.claims.push((c.name.clone(), c.kind, service.clone()));
                }
            }
            for (key, r) in &preset.records {
                if let Some(content) = fill(&r.content, &values) {
                    let raw = fill_raw(&r.content, &values, &self.secrets);
                    self.recs.insert(
                        format!("{service}-{key}"),
                        Rec {
                            owner: service.clone(),
                            name: r.name.clone(),
                            kind: r.kind.clone(),
                            content,
                            raw,
                            ttl: r.ttl.unwrap_or(1),
                            priority: r.priority,
                            proxied: r.proxied,
                        },
                    );
                }
            }
        }
    }

    /// Each workload's hosts: claimed always, rendered from its `ingress`.
    fn hosts(&mut self, config: &Value, root: &Path, zone: &str) {
        self.prefixes.push("host-".into());
        let units = workloads(root, zone);
        let overrides = config["hosts"].as_object().cloned().unwrap_or_default();
        for w in overrides.keys() {
            if !units.contains_key(w) && config["apex"].as_str() != Some(w) {
                self.problems
                    .push(format!("hosts.{w}: no workload at workloads/{zone}/{w}"));
            }
        }
        for (w, list) in &overrides {
            for h in list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !HOST.is_match(h) {
                    self.problems
                        .push(format!("hosts.{w}: `{h}` is not a host name"));
                } else if h == zone || h.ends_with(&format!(".{zone}")) {
                    self.problems
                        .push(format!("hosts.{w}: `{h}` must be relative to the zone"));
                }
            }
        }
        let assigned = assignments(config, &units, zone);
        for (w, hosts) in &assigned {
            let mut keys: BTreeMap<String, &str> = BTreeMap::new();
            for h in hosts {
                let key = format!("host-{w}-{}", label(h));
                if let Some(first) = keys.insert(key.clone(), h) {
                    self.problems.push(format!(
                        "hosts.{w}: `{first}` and `{h}` would share the record key `{key}`"
                    ));
                }
            }
            for h in hosts {
                self.claims
                    .push((h.clone(), ClaimType::Address, format!("workload {w}")));
            }
            if units.get(w) != Some(&true) {
                continue;
            }
            let own = workload_config(root, zone, w);
            let site = own.as_ref().is_some_and(crate::static_site::is_static);
            if site && hosts.is_empty() {
                self.problems.push(format!(
                    "workloads/{zone}/{w}: a static site needs a host; the apex assigns it none"
                ));
            }
            let Some(ing) = ingress(root, zone, w) else {
                continue;
            };
            if site {
                self.problems.push(format!(
                    "workloads/{zone}/{w}: ingress and static_site cannot both serve a workload"
                ));
                continue;
            }
            match check_ingress(&ing) {
                Ok((kind, content, proxied)) => {
                    for h in hosts {
                        self.recs.insert(
                            format!("host-{w}-{}", label(h)),
                            Rec {
                                owner: format!("workload {w}"),
                                name: h.clone(),
                                kind: kind.clone(),
                                content: escape(&content),
                                raw: Some(content.clone()),
                                ttl: 1,
                                priority: None,
                                proxied: Some(proxied),
                            },
                        );
                    }
                }
                Err(e) => self
                    .problems
                    .push(format!("workloads/{zone}/{w}: ingress {e}")),
            }
        }
    }

    /// The explicit `records`: drops, field-by-field overrides, and new records.
    fn records(&mut self, config: &Value) {
        for (key, v) in config["records"].as_object().into_iter().flatten() {
            let at = format!("records.{key}");
            if v == &Value::Bool(false) {
                if self.recs.remove(key).is_none() {
                    self.problems
                        .push(format!("{at}: drops no preset or host record"));
                }
                continue;
            }
            let Some(o) = v.as_object() else {
                self.problems
                    .push(format!("{at}: must be a record or false"));
                continue;
            };
            if let Some(r) = self.recs.get_mut(key) {
                r.merge(o);
                if let Some(c) = o.get("content") {
                    r.raw = raw_of(c, &self.secrets);
                }
                continue;
            }
            if let Some(p) = self.prefixes.iter().find(|p| key.starts_with(p.as_str())) {
                self.problems.push(format!(
                    "{at}: `{}` names no such record",
                    p.trim_end_matches('-')
                ));
                continue;
            }
            let (Some(name), Some(kind), Some(content)) = (
                o.get("name").and_then(Value::as_str),
                o.get("type").and_then(Value::as_str),
                o.get("content").map(content_of),
            ) else {
                self.problems
                    .push(format!("{at}: a new record needs name, type and content"));
                continue;
            };
            let raw = o.get("content").and_then(|c| raw_of(c, &self.secrets));
            let mut rec = Rec {
                owner: at.clone(),
                name: name.to_string(),
                kind: kind.to_string(),
                content,
                raw,
                ttl: 1,
                priority: None,
                proxied: None,
            };
            rec.merge(o);
            self.recs.insert(key.clone(), rec);
        }
    }

    /// Rejects claims made twice and CNAMEs that share a name; renders and checks every record.
    fn finish(mut self, zone: &str) -> (Map<String, Value>, Vec<String>) {
        let mut by_name: BTreeMap<&str, Vec<(&str, &Rec)>> = BTreeMap::new();
        for (key, r) in &self.recs {
            if let Some(c) = implied_claim(&r.kind, r.raw.as_deref()) {
                self.claims.push((r.name.clone(), c, r.owner.clone()));
            }
            by_name.entry(&r.name).or_default().push((key, r));
        }
        for (name, recs) in by_name.iter().filter(|(n, rs)| **n != "@" && rs.len() > 1) {
            for (key, _) in recs.iter().filter(|(_, r)| r.kind == "CNAME") {
                let others: Vec<&str> = recs.iter().map(|(k, _)| *k).filter(|k| k != key).collect();
                self.problems.push(format!(
                    "{name} holds a CNAME ({key}), which shares its name with no other record: {}",
                    others.join(", ")
                ));
            }
        }
        let mut seen: BTreeMap<(String, ClaimType), String> = BTreeMap::new();
        for (name, kind, owner) in std::mem::take(&mut self.claims) {
            match seen.get(&(name.clone(), kind)) {
                Some(first) if first != &owner => self.problems.push(format!(
                    "{name} {} is claimed by {first} and {owner}",
                    kind.word()
                )),
                Some(_) => {}
                None => {
                    seen.insert((name, kind), owner);
                }
            }
        }
        let mut out = Map::new();
        for (key, r) in std::mem::take(&mut self.recs) {
            match check_record(&r, zone) {
                Ok(()) => {
                    out.insert(
                        key,
                        json!({
                            "name": fqdn(&r.name, zone),
                            "type": r.kind,
                            "content": r.content,
                            "ttl": r.ttl,
                            "priority": r.priority,
                            "proxied": r.proxied.unwrap_or(false),
                        }),
                    );
                }
                Err(e) => self.problems.push(format!("record `{key}`: {e}")),
            }
        }
        (out, self.problems)
    }
}

impl Rec {
    /// Overrides the fields `o` sets.
    fn merge(&mut self, o: &Map<String, Value>) {
        if let Some(n) = o.get("name").and_then(Value::as_str) {
            self.name = n.to_string();
        }
        if let Some(t) = o.get("type").and_then(Value::as_str) {
            if t != "MX" && !o.contains_key("priority") {
                self.priority = None;
            }
            self.kind = t.to_string();
        }
        if let Some(c) = o.get("content") {
            self.content = content_of(c);
        }
        if let Some(t) = o.get("ttl").and_then(Value::as_i64) {
            self.ttl = t;
        }
        match o.get("priority") {
            Some(Value::Null) => self.priority = None,
            Some(p) => self.priority = p.as_i64().or(self.priority),
            None => {}
        }
        if let Some(p) = o.get("proxied").and_then(Value::as_bool) {
            self.proxied = Some(p);
        }
    }
}

/// An explicit record's content as published, when known.
fn raw_of(c: &Value, secrets: &BTreeMap<String, String>) -> Option<String> {
    as_secret(c).map_or_else(
        || c.as_str().map(str::to_string),
        |k| secrets.get(k).cloned(),
    )
}

/// An explicit record's content as an HCL template.
fn content_of(c: &Value) -> String {
    as_secret(c).map_or_else(|| escape(c.as_str().unwrap_or_default()), secret_ref)
}

/// One zone's records, keyed by record key, and its problems.
fn expand_zone(
    config: &Value,
    home: &Path,
    root: &Path,
    zone: &str,
    secrets: &BTreeMap<String, String>,
) -> (Map<String, Value>, Vec<String>) {
    let mut e = Expansion {
        secrets: secrets.clone(),
        ..Expansion::default()
    };
    bad_secrets(config, "config", &mut e.problems);
    e.services(config, home);
    e.hosts(config, root, zone);
    e.records(config);
    e.finish(zone)
}

/// Every zone's records, keyed `<zone>:<record key>`, each with its `zone`, and their problems.
#[must_use]
pub fn expand(config: &Value, home: &Path, site: &Site) -> (Map<String, Value>, Vec<String>) {
    expand_with(config, home, site, &BTreeMap::new())
}

/// A derived block's items and their problems, by its expander.
#[must_use]
pub fn derive(
    expander: &str,
    config: &Value,
    home: &Path,
    site: &Site,
) -> (Map<String, Value>, Vec<String>) {
    match expander {
        EXPANDER => expand(config, home, site),
        crate::static_site::RULESETS => {
            (crate::static_site::rulesets(config, &site.root), Vec::new())
        }
        crate::static_site::BUCKET => (crate::static_site::bucket(config, site), Vec::new()),
        crate::static_site::DOMAINS => crate::static_site::domains(config, site),
        _ => (Map::new(), vec![format!("unknown expander `{expander}`")]),
    }
}

/// The problems once secret-backed records are classified by their decrypted
/// content; messages name records and claims, never values.
#[must_use]
pub fn check_decrypted(
    config: &Value,
    home: &Path,
    site: &Site,
    secrets: &BTreeMap<String, String>,
) -> Vec<String> {
    expand_with(config, home, site, secrets).1
}

fn expand_with(
    config: &Value,
    home: &Path,
    site: &Site,
    secrets: &BTreeMap<String, String>,
) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::new();
    let mut problems = Vec::new();
    for zone in zones(config) {
        let (recs, ps) = expand_zone(&config["zones"][&zone], home, &site.root, &zone, secrets);
        problems.extend(ps.into_iter().map(|p| format!("zones.{zone}: {p}")));
        for (key, mut item) in recs {
            item["zone"] = Value::String(zone.clone());
            out.insert(format!("{zone}:{key}"), item);
        }
    }
    (out, problems)
}

/// A record name's fully qualified form in `zone`.
pub(crate) fn fqdn(name: &str, zone: &str) -> String {
    if name == "@" {
        zone.to_string()
    } else {
        format!("{name}.{zone}")
    }
}

fn check_record(r: &Rec, zone: &str) -> Result<(), String> {
    if r.name.ends_with(&format!(".{zone}")) || r.name == zone {
        return Err(format!("name `{}` must be relative to the zone", r.name));
    }
    if !HOST.is_match(&r.name) {
        return Err(format!("name `{}` is not a DNS name", r.name));
    }
    if !RECORD_TYPES.contains(&r.kind.as_str()) {
        return Err(format!("type `{}` is not one of {RECORD_TYPES:?}", r.kind));
    }
    if r.ttl != 1 && !(60..=86_400).contains(&r.ttl) {
        return Err("ttl must be 1 or 60 to 86400".into());
    }
    match (r.kind.as_str(), r.priority) {
        ("MX", None) => return Err("an MX record needs a priority".into()),
        ("MX", Some(p)) if !(0..=65_535).contains(&p) => {
            return Err("priority must be 0 to 65535".into());
        }
        ("MX", _) | (_, None) => {}
        _ => return Err("only MX records take a priority".into()),
    }
    if r.proxied == Some(true) && !ADDRESS_TYPES.contains(&r.kind.as_str()) {
        return Err("only A, AAAA and CNAME records can be proxied".into());
    }
    if r.content.is_empty() {
        return Err("content is empty".into());
    }
    Ok(())
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
            if !apex.join(crate::config::CONFIG).is_file() {
                out.push((format!("{rel}/{n}"), "an apex holds config.yaml".into()));
            }
            for own in [crate::config::SCHEMA, "modules", "unit.yaml"] {
                if apex.join(own).exists() {
                    out.push((
                        format!("{rel}/{n}/{own}"),
                        format!("the apex uses {CLOUDFLARE}'s schema and modules, never its own"),
                    ));
                }
            }
        } else if !NAME.is_match(&n) {
            out.push((
                format!("{rel}/{n}"),
                "a workload's name is lowercase words joined by dashes".into(),
            ));
        } else if !e.path().join(crate::config::CONFIG).is_file()
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
        if !crate::static_site::sites(root, &zone).is_empty() {
            for p in crate::static_site::APEX_RULE_PERMISSIONS {
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
    let cf: Option<Value> = fs::read_to_string(root.join(CLOUDFLARE).join(crate::config::CONFIG))
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
            if ZONE.is_match(name) {
                zone_dir(&rel, &root.join(&rel), &mut out);
            } else {
                out.push((rel, format!("`{name}` is not a zone name")));
            }
        }
        let zones: Vec<&str> = dirs
            .iter()
            .map(String::as_str)
            .filter(|n| ZONE.is_match(n))
            .collect();
        crate::static_site::check_buckets(root, &zones, &mut out);
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
        if !ZONE.is_match(name) || !listed.contains(name.as_str()) {
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
                .join(crate::config::CONFIG)
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
        crate::static_site::check_tokens(root, &cf, zone, &mut out);
    }
    crate::static_site::check_buckets(root, &zones, &mut out);
    out
}

/// Each zone's handed-off values, from `infra/cloudflare`'s `tofu show -json` state;
/// a zone lacks a key until state holds it.
#[must_use]
pub fn handoff(
    state_json: &str,
    zones: &[String],
    today: Day,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let state = serde_json::from_str::<Value>(state_json).unwrap_or(Value::Null);
    let outputs = &state["values"]["outputs"];
    let mut out = BTreeMap::new();
    for zone in zones {
        let mut values = BTreeMap::new();
        if let Some(id) = outputs["zone_ids"]["value"][zone].as_str() {
            values.insert("zone_id".to_string(), id.to_string());
        }
        if let Some((_, v)) = live_token(outputs, "tokens", &token_name(zone), today) {
            values.insert("api_token".to_string(), v);
        }
        out.insert(zone.clone(), values);
    }
    out
}

/// The live generation of `token` in an output map keyed by generation, and its value there.
pub(crate) fn live_token(
    outputs: &Value,
    output: &str,
    token: &str,
    today: Day,
) -> Option<(Generation, String)> {
    outputs[output]["value"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let (name, g) = Generation::parse(k)?;
            (name == token && g.not_before <= today && today < g.expires_on)
                .then_some((g, v.as_str()?.to_string()))
        })
        .max_by_key(|(g, _)| *g)
}

#[cfg(test)]
mod tests {
    use super::{Resolved, fill, handoff, label};
    use std::collections::BTreeMap;

    #[test]
    fn templates_escape_literals_and_reference_secrets() {
        let mut v = BTreeMap::new();
        v.insert("a".to_string(), Resolved::Literal("x${y}".into()));
        v.insert("s".to_string(), Resolved::Secret("k".into()));
        assert_eq!(
            fill("p=${a};${s}%{", &v).as_deref(),
            Some("p=x$${y};${var.secrets[\"k\"]}%%{")
        );
        assert_eq!(fill("${missing}", &v), None);
    }

    #[test]
    fn labels_are_kebab() {
        assert_eq!(label("@"), "apex");
        assert_eq!(label("www"), "www");
        assert_eq!(label("a.b"), "a-b");
    }

    #[test]
    fn the_live_generation_is_the_latest_started() {
        let d = |s| crate::generations::day(s).expect("day");
        let state = serde_json::json!({ "values": { "outputs": {
            "zone_ids": { "value": { "z.com": "zid" } },
            "tokens": { "value": {
                "apex-z.com@2026-08-04..2026-11-09": "old",
                "apex-z.com@2026-11-02..2027-02-07": "new",
                "apex-other.com@2026-11-02..2027-02-07": "other"
            } }
        } } })
        .to_string();
        let z = ["z.com".to_string()];
        let inside = handoff(&state, &z, d("2026-11-05"));
        assert_eq!(inside["z.com"]["api_token"], "new");
        assert_eq!(inside["z.com"]["zone_id"], "zid");
        assert_eq!(
            handoff(&state, &z, d("2026-11-01"))["z.com"]["api_token"],
            "old"
        );
        assert!(!handoff(&state, &z, d("2027-03-01"))["z.com"].contains_key("api_token"));
        assert!(handoff("{}", &z, 0)["z.com"].is_empty());
    }
}
