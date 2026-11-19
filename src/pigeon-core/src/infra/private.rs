//! Private zones by alias; see .claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md §1.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::infra::manifest::name;

/// The manifest's raw units with each private zone resolved from `private`, or dropped with its sites; and the problems.
#[must_use]
pub fn resolve(
    units: BTreeMap<String, Value>,
    private: &Map<String, Value>,
) -> (BTreeMap<String, Value>, Vec<(String, String)>) {
    let mut problems = Vec::new();
    let mut resolved: BTreeMap<String, String> = BTreeMap::new();
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    for (id, unit) in &units {
        if unit.get("private") != Some(&Value::Bool(true)) {
            continue;
        }
        let alias = name(id).to_string();
        let at = format!("config.yaml: units.{id}");
        let mut bad = false;
        if unit.get("services").is_some() || unit.get("records").is_some() {
            problems.push((
                at.clone(),
                "a private zone holds no services or records; they come from the private store"
                    .into(),
            ));
            bad = true;
        }
        if alias.contains('.') {
            problems.push((at, "a private zone's key is an alias, with no dot".into()));
            bad = true;
        }
        if bad {
            dropped.insert(alias);
            continue;
        }
        let zone = private
            .get(&alias)
            .and_then(|e| e.get("zone"))
            .and_then(Value::as_str)
            .filter(|z| z.contains('.'));
        if let Some(z) = zone {
            resolved.insert(alias, z.to_string());
        } else {
            if private.get(&alias).is_some() {
                problems.push(("--private".into(), format!("{alias}: needs its zone")));
            }
            dropped.insert(alias);
        }
    }
    let mut out = BTreeMap::new();
    for (id, mut unit) in units {
        if unit.get("private") == Some(&Value::Bool(true)) {
            let alias = name(&id).to_string();
            let Some(zone) = resolved.get(&alias) else {
                continue;
            };
            let entry = &private[&alias];
            let mut map = Map::new();
            if let Some(kind) = unit.get("kind") {
                map.insert("kind".into(), kind.clone());
            }
            for key in ["services", "records"] {
                map.insert(
                    key.into(),
                    entry
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Map::new())),
                );
            }
            let area = id.rsplit_once('/').map_or("zones", |(a, _)| a);
            out.insert(format!("{area}/{zone}"), Value::Object(map));
            continue;
        }
        if let Some(z) = unit.get("zone").and_then(Value::as_str).map(str::to_string) {
            if dropped.contains(&z) {
                continue;
            }
            if let Some(real) = resolved.get(&z) {
                unit["zone"] = Value::String(real.clone());
            }
        }
        out.insert(id, unit);
    }
    (out, problems)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn units(v: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(v).expect("units")
    }

    fn map(v: &Value) -> Map<String, Value> {
        v.as_object().expect("object").clone()
    }

    fn blog() -> BTreeMap<String, Value> {
        units(json!({
            "zones/blog": {"kind": "cloudflare/zone", "private": true},
            "sites/blog": {"kind": "cloudflare/static-site", "zone": "blog", "hosts": ["@"]},
        }))
    }

    #[test]
    fn an_entry_renames_the_zone_and_points_its_sites_at_it() {
        let private = map(&json!({"blog": {"zone": "example.com", "records": {
            "a": {"name": "x", "type": "TXT", "content": "y"}}}}));
        let (out, problems) = resolve(blog(), &private);
        assert_eq!(problems, []);
        assert!(!out.contains_key("zones/blog"));
        assert_eq!(out["zones/example.com"]["services"], json!({}));
        assert_eq!(
            out["zones/example.com"]["records"],
            json!({"a": {"name": "x", "type": "TXT", "content": "y"}})
        );
        assert!(out["zones/example.com"].get("private").is_none());
        assert_eq!(out["sites/blog"]["zone"], "example.com");
    }

    #[test]
    fn without_an_entry_the_zone_and_its_sites_leave() {
        let (out, problems) = resolve(blog(), &Map::new());
        assert_eq!(problems, []);
        assert!(out.is_empty());
    }

    #[test]
    fn a_private_unit_holds_no_records_and_its_key_has_no_dot() {
        let input = units(json!({
            "zones/blog": {"kind": "cloudflare/zone", "private": true, "records": {}},
            "zones/a.b": {"kind": "cloudflare/zone", "private": true},
        }));
        let (out, problems) = resolve(input, &Map::new());
        assert!(out.is_empty());
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|(_, m)| m
            == "a private zone holds no services or records; they come from the private store"));
        assert!(
            problems
                .iter()
                .any(|(_, m)| m == "a private zone's key is an alias, with no dot")
        );
    }

    #[test]
    fn other_units_pass_untouched() {
        let input = units(json!({
            "zones/pigeon.dev": {"kind": "cloudflare/zone", "services": {}, "records": {}},
            "sites/docs": {"zone": "pigeon.dev"},
        }));
        let private = map(&json!({"blog": {"zone": "example.com"}}));
        let (out, problems) = resolve(input.clone(), &private);
        assert_eq!(problems, []);
        assert_eq!(out, input);
    }
}
