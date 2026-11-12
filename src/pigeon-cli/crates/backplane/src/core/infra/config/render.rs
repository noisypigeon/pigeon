//! A config unit's `main.tf.json`, rendered from its schema's `x-cobot` and its config.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{
    Block, ConfigUnit, Disk, Expr, ZONE_ALIAS, blocks, declared_credentials, declared_secrets,
    has_handoff, parse, x_cobot,
};
use crate::cobot::apex;
use crate::cobot::generations::{self, Day, Generation};
use crate::core::infra::providers::Allowlist;

/// Escapes template sequences so a literal string is never evaluated.
pub(crate) fn literal(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("${", "$${").replace("%{", "%%{")),
        Value::Array(a) => Value::Array(a.iter().map(literal).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), literal(v))).collect()),
        other => other.clone(),
    }
}

pub(crate) fn lookup<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, p| v.get(p))
}

pub(crate) fn dotted(path: &[String]) -> String {
    path.iter().fold(String::new(), |mut out, p| {
        out.push('.');
        out.push_str(p);
        out
    })
}

/// Renders an expression; `zone` indexes `var.handoff` (`each.key`, `each.value.zone`).
pub(crate) fn render_value(value: &Value, config: &Value, zone: &str) -> Value {
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

pub(crate) fn number(value: &Value, config: &Value) -> Option<i64> {
    match value {
        Value::String(s) => match parse(s) {
            Some(Expr::Config(p)) => lookup(config, &p).and_then(Value::as_i64),
            _ => None,
        },
        other => other.as_i64(),
    }
}

/// A rotating block's period and overlap, both checked.
pub(crate) fn rotation(block: &Block, config: &Value) -> Result<(i64, i64), String> {
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
pub(crate) fn item_keys(block: &Block, config: &Value) -> Result<Vec<String>, String> {
    let Some(Expr::Item(path)) = block.for_each.as_deref().and_then(parse) else {
        return Err(format!(
            "`{}`: for_each must be `$item` or `$item.<field>`",
            block.name
        ));
    };
    // An absent list is empty; see .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md.
    let Some(over) = config.get(&block.over) else {
        return Ok(Vec::new());
    };
    let Some(items) = over.as_array() else {
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
pub(crate) fn extras(unit: &ConfigUnit, root: &mut Value) {
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
pub(crate) fn providers(
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
    disk: &Disk,
    state_file: &str,
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
                disk.modules
                    .get(&b.module)
                    .map(|m| m.source.clone())
                    .unwrap_or_default(),
            ),
        );
        if b.derived.is_some() {
            let items = disk.derived.get(&b.name).cloned().unwrap_or_default();
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
        "backend": { "local": { "path": state_file } }
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
