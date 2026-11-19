//! Adopting existing resources by name: an `OpenTofu` `import` block for each resource a unit
//! declares that its account already holds under that name, so no ID enters the repository;
//! see `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::workers::cloudflare::Api;

/// A resource kind infra can find by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A D1 database.
    D1,
    /// An R2 bucket.
    Bucket,
    /// A Queue.
    Queue,
    /// An Access application.
    AccessApp,
    /// A reusable Access policy.
    AccessPolicy,
    /// An Access service token.
    ServiceToken,
}

impl Kind {
    /// The kind a catalog module creates, by the module's name, and its resource address.
    #[must_use]
    pub fn of_module(name: &str) -> Option<(Self, &'static str)> {
        Some(match name {
            "d1-database" => (Self::D1, "cloudflare_d1_database.this"),
            "r2-bucket" => (Self::Bucket, "cloudflare_r2_bucket.this"),
            "queue" => (Self::Queue, "cloudflare_queue.this"),
            "access-application" => (
                Self::AccessApp,
                "cloudflare_zero_trust_access_application.this",
            ),
            "access-policy" => (
                Self::AccessPolicy,
                "cloudflare_zero_trust_access_policy.this",
            ),
            "access-service-token" => (
                Self::ServiceToken,
                "cloudflare_zero_trust_access_service_token.this",
            ),
            _ => return None,
        })
    }

    /// The `OpenTofu` import ID of the resource `id`, named `name`, on `account`.
    #[must_use]
    pub fn import_id(self, account: &str, id: &str, name: &str, jurisdiction: &str) -> String {
        match self {
            Self::D1 | Self::Queue | Self::AccessPolicy => format!("{account}/{id}"),
            Self::Bucket => format!("{account}/{name}/{jurisdiction}"),
            Self::AccessApp | Self::ServiceToken => format!("accounts/{account}/{id}"),
        }
    }
}

/// One resource found: its name and ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its name.
    pub name: String,
    /// Its ID; a bucket's is its name.
    pub id: String,
}

/// One DNS record in a zone.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Its ID.
    pub id: String,
    /// Its fully qualified name.
    pub name: String,
    /// Its type.
    pub kind: String,
    /// Its content; empty for a record with only `data`.
    pub content: String,
    /// SRV's or CAA's fields.
    pub data: Option<Value>,
}

/// Lists an account's resources of one kind.
pub trait Lookup {
    /// Every resource of `kind`; for a bucket, in `jurisdiction`.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    fn list(&self, kind: Kind, jurisdiction: &str) -> Result<Vec<Found>, String>;

    /// Zone `zone`'s ID and every DNS record in it.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    fn records(&self, zone: &str) -> Result<(String, Vec<Record>), String> {
        let _ = zone;
        Err("this lookup lists no DNS records".into())
    }
}

/// The Cloudflare API, on one account.
pub struct Cloudflare<'a> {
    /// The API.
    pub api: &'a Api,
    /// The account.
    pub account: &'a str,
}

fn items(v: &Value) -> Vec<Value> {
    v.as_array()
        .or_else(|| v["buckets"].as_array())
        .cloned()
        .unwrap_or_default()
}

impl Lookup for Cloudflare<'_> {
    fn list(&self, kind: Kind, jurisdiction: &str) -> Result<Vec<Found>, String> {
        let a = self.account;
        let (path, name, id) = match kind {
            Kind::D1 => (
                format!("/accounts/{a}/d1/database?per_page=1000"),
                "name",
                "uuid",
            ),
            Kind::Bucket => (
                format!("/accounts/{a}/r2/buckets?per_page=1000"),
                "name",
                "name",
            ),
            Kind::Queue => (
                format!("/accounts/{a}/queues?per_page=1000"),
                "queue_name",
                "queue_id",
            ),
            Kind::AccessApp => (format!("/accounts/{a}/access/apps"), "name", "id"),
            Kind::AccessPolicy => (format!("/accounts/{a}/access/policies"), "name", "id"),
            Kind::ServiceToken => (format!("/accounts/{a}/access/service_tokens"), "name", "id"),
        };
        let v = if kind == Kind::Bucket && jurisdiction != "default" {
            self.api
                .get_with(&path, &[("cf-r2-jurisdiction", jurisdiction)])?
        } else {
            self.api.get(&path)?
        };
        Ok(items(&v)
            .iter()
            .filter_map(|x| {
                Some(Found {
                    name: x[name].as_str()?.to_string(),
                    id: x[id].as_str()?.to_string(),
                })
            })
            .collect())
    }

    fn records(&self, zone: &str) -> Result<(String, Vec<Record>), String> {
        let id = self.api.zone_id(zone)?;
        let mut all = Vec::new();
        for page in 1..=1000 {
            let v = self
                .api
                .get(&format!("/zones/{id}/dns_records?per_page=100&page={page}"))?;
            let batch = items(&v);
            if batch.is_empty() {
                let records = all
                    .iter()
                    .filter_map(|x: &Value| {
                        Some(Record {
                            id: x["id"].as_str()?.to_string(),
                            name: x["name"].as_str()?.to_string(),
                            kind: x["type"].as_str()?.to_string(),
                            content: x["content"].as_str().unwrap_or_default().to_string(),
                            data: x.get("data").filter(|d| d.is_object()).cloned(),
                        })
                    })
                    .collect();
                return Ok((id, records));
            }
            all.extend(batch);
        }
        Err("the zone holds more than 100000 DNS records; adopt it in parts".into())
    }
}

/// How `s` matches `name`: exactly, or as `name-<suffix>` with a suffix of lowercase letters
/// and digits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Match {
    Exact,
    Suffixed(String),
}

fn named(s: &str, name: &str) -> Option<Match> {
    if s == name {
        return Some(Match::Exact);
    }
    let suffix = s.strip_prefix(name)?.strip_prefix('-')?;
    (!suffix.is_empty()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
    .then(|| Match::Suffixed(suffix.to_string()))
}

/// A module input's literal string: the value itself, or, under `for_each`, the item's key or
/// one of its fields.
fn literal(v: &Value, key: Option<&str>, item: &Value) -> Option<String> {
    let s = v.as_str()?;
    if s == "${each.key}" {
        return key.map(str::to_string);
    }
    if let Some(field) = s
        .strip_prefix("${each.value.")
        .and_then(|f| f.strip_suffix('}'))
    {
        return item[field].as_str().map(str::to_string);
    }
    (!s.contains("${")).then(|| s.to_string())
}

/// The block and key a `${module.<block>[<key>].result}` suffix input names; `<key>` may be
/// `each.value.name`, then the item's key.
fn suffix_block(v: &Value, key: Option<&str>) -> Option<(String, String)> {
    let s = v
        .as_str()?
        .strip_prefix("${module.")?
        .strip_suffix("].result}")?;
    let (block, k) = s.split_once('[')?;
    let k = match k {
        "each.value.name" | "each.key" => key?.to_string(),
        quoted => quoted.trim_matches('"').to_string(),
    };
    Some((block.to_string(), k))
}

fn address(block: &str, key: Option<&str>, resource: &str) -> String {
    match key {
        Some(k) => format!("module.{block}[\"{k}\"].{resource}"),
        None => format!("module.{block}.{resource}"),
    }
}

/// The `import` blocks for the modules of a rendered `main.tf.json` whose resources `lookup`
/// finds by name on `account`. A random suffix a bucket was found with is imported into its
/// suffix block. A name matching none is left to be created. DNS records are matched in their
/// zone by name, type, and content or data.
///
/// # Errors
///
/// A message for a lookup that fails, or a name matching more than one resource.
pub fn imports(rendered: &Value, account: &str, lookup: &dyn Lookup) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let Some(modules) = rendered["module"].as_object() else {
        return Ok(out);
    };
    let mut zones: BTreeMap<String, (String, Vec<Record>)> = BTreeMap::new();
    for (block, m) in modules {
        let source = m["source"].as_str().unwrap_or_default();
        let module = source
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        if module == "dns-record" {
            dns_imports(block, m, lookup, &mut zones, &mut out)?;
            continue;
        }
        let Some((kind, resource)) = Kind::of_module(module) else {
            continue;
        };
        let jurisdiction = m["jurisdiction"].as_str().unwrap_or("eu");
        let keys: Vec<Option<String>> = match m["for_each"].as_object() {
            Some(items) => items.keys().cloned().map(Some).collect(),
            None => vec![None],
        };
        let all = lookup.list(kind, jurisdiction)?;
        for key in keys {
            let item = key
                .as_deref()
                .map_or(Value::Null, |k| m["for_each"][k].clone());
            let Some(name) = literal(&m["name"], key.as_deref(), &item) else {
                continue;
            };
            let suffixed = kind == Kind::Bucket && !m["suffix"].is_null();
            let hits: Vec<(&Found, Match)> = all
                .iter()
                .filter_map(|f| {
                    let m = if suffixed {
                        named(&f.name, &name)?
                    } else {
                        (f.name == name).then_some(Match::Exact)?
                    };
                    Some((f, m))
                })
                .collect();
            let (found, suffix) = match hits.as_slice() {
                [] => continue,
                [one] => one.clone(),
                _ => {
                    return Err(format!(
                        "`{name}` names {} {kind:?} resources; adopt one by renaming the others",
                        hits.len()
                    ));
                }
            };
            out.push(json!({
                "to": address(block, key.as_deref(), resource),
                "id": kind.import_id(account, &found.id, &found.name, jurisdiction),
            }));
            if let (Match::Suffixed(sfx), Some((sblock, skey))) =
                (suffix, suffix_block(&m["suffix"], key.as_deref()))
            {
                out.push(json!({
                    "to": address(&sblock, Some(&skey), "random_string.this"),
                    "id": sfx,
                }));
            }
        }
    }
    Ok(out)
}

/// The import blocks for a dns-record block's items that their zone already holds; each zone is
/// listed once.
fn dns_imports(
    block: &str,
    m: &Value,
    lookup: &dyn Lookup,
    zones: &mut BTreeMap<String, (String, Vec<Record>)>,
    out: &mut Vec<Value>,
) -> Result<(), String> {
    let Some(items) = m["for_each"].as_object() else {
        return Ok(());
    };
    for (key, item) in items {
        let (Some(zone), Some(name), Some(kind)) = (
            item["zone"].as_str(),
            item["name"].as_str(),
            item["type"].as_str(),
        ) else {
            continue;
        };
        if !zones.contains_key(zone) {
            let listed = lookup.records(zone)?;
            zones.insert(zone.to_string(), listed);
        }
        let (zone_id, records) = &zones[zone];
        let hits: Vec<&Record> = records
            .iter()
            .filter(|r| r.name.eq_ignore_ascii_case(name) && r.kind == kind && matches(item, r))
            .collect();
        match hits.as_slice() {
            [] => {}
            [r] => out.push(json!({
                "to": address(block, Some(key), "cloudflare_dns_record.this"),
                "id": format!("{zone_id}/{}", r.id),
            })),
            _ => {
                return Err(format!(
                    "record `{key}` matches {} live records; adopt it by removing the others",
                    hits.len()
                ));
            }
        }
    }
    Ok(())
}

/// Whether the live record `r` holds what `item` declares.
fn matches(item: &Value, r: &Record) -> bool {
    if let Some(d) = item["data"].as_object() {
        return r
            .data
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|rd| {
                d.iter()
                    .all(|(k, v)| rd.get(k).is_some_and(|x| same_field(v, x)))
            });
    }
    match item["content"].as_str() {
        Some(c) if c.contains("${var.secrets[") => true,
        Some(c) => same_content(
            &r.kind,
            &c.replace("$${", "${").replace("%%{", "%{"),
            &r.content,
        ),
        None => false,
    }
}

/// Whether two data fields are equal: strings ignoring a trailing dot and case, numbers by value.
fn same_field(a: &Value, b: &Value) -> bool {
    match (a.as_str(), b.as_str()) {
        (Some(x), Some(y)) => x
            .trim_end_matches('.')
            .eq_ignore_ascii_case(y.trim_end_matches('.')),
        _ => a.as_f64().is_some() && a.as_f64() == b.as_f64(),
    }
}

/// Whether two record contents of type `kind` are the same.
fn same_content(kind: &str, a: &str, b: &str) -> bool {
    match kind {
        "TXT" => txt(a) == txt(b),
        "CNAME" | "MX" | "NS" => a
            .trim_end_matches('.')
            .eq_ignore_ascii_case(b.trim_end_matches('.')),
        "AAAA" => match (
            a.parse::<std::net::Ipv6Addr>(),
            b.parse::<std::net::Ipv6Addr>(),
        ) {
            (Ok(x), Ok(y)) => x == y,
            _ => a == b,
        },
        _ => a == b,
    }
}

/// A TXT content without its outer quotes and the joints between its quoted chunks.
fn txt(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].replace("\" \"", "")
    } else {
        s.to_string()
    }
}
