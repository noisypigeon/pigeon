//! What the engines decide without running anything: the runner's credentials, a plan's
//! attribute diff and summary, and the handoff's placeholders; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use crate::cobot::UnitPlan;
use crate::cobot::{apex, static_site};

/// The runner's `json` with each requested `cloudflare` credential it does not name filled
/// from the run's own `token`, per
/// `.claude/decisions/2026-11-05-runtime-instance-config-kind-units.md` §3; `json` itself when
/// there is nothing to fill or it is not an object.
#[must_use]
pub fn fill_credentials(
    json: Option<&str>,
    providers: &BTreeMap<String, String>,
    token: Option<&str>,
) -> Option<String> {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return json.map(str::to_string);
    };
    let Ok(mut all) = serde_json::from_str::<Map<String, Value>>(json.unwrap_or("{}")) else {
        return json.map(str::to_string);
    };
    for (name, provider) in providers {
        if provider == "cloudflare" && !all.contains_key(name) {
            all.insert(name.clone(), json!({ "api_token": token }));
        }
    }
    Some(Value::Object(all).to_string())
}

/// From the runner's `json`, exactly the fields `unit` references of each credential it
/// requests (`wanted`); other credentials and fields are never read. Never echoes a value.
///
/// # Errors
///
/// A message when `json` is not an object, or a wanted credential or field is missing or not
/// a string.
pub fn pick_credentials(
    json: Option<&str>,
    wanted: &BTreeMap<String, BTreeSet<String>>,
    unit: &str,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let all: Map<String, Value> = serde_json::from_str(json.unwrap_or("{}"))
        .map_err(|_| "must be a JSON object mapping credential names to fields".to_string())?;
    let mut out = BTreeMap::new();
    for (name, fields) in wanted {
        let entry = all.get(name).filter(|e| e.is_object()).ok_or_else(|| {
            format!(
                "{unit} requests credential `{name}`, which the runner did not supply as an object"
            )
        })?;
        let mut picked = BTreeMap::new();
        for field in fields {
            let value = entry.get(field).and_then(Value::as_str).ok_or_else(|| {
                format!(
                    "{unit} needs `{name}.{field}`, which the runner did not supply as a string"
                )
            })?;
            picked.insert(field.clone(), value.to_string());
        }
        out.insert(name.clone(), picked);
    }
    Ok(out)
}

/// `values` with an inert value for each of `keys` it lacks, as a plan uses while the
/// handoff is pending.
#[must_use]
pub fn placeholders(
    mut values: BTreeMap<String, String>,
    keys: &[&str],
) -> BTreeMap<String, String> {
    for k in keys {
        let inert = match *k {
            "zone_id" => apex::PLACEHOLDER_ZONE_ID,
            "account_id" => static_site::PLACEHOLDER_ACCOUNT_ID,
            _ => apex::PLACEHOLDER_TOKEN,
        };
        values.entry((*k).into()).or_insert_with(|| inert.into());
    }
    values
}

/// One attribute a planned resource change sets, as `show` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrChange {
    /// The resource's address.
    pub address: String,
    /// What happens to it: `update`, `create`, `delete`, `replace` or `read`.
    pub action: String,
    /// The attribute, top-level.
    pub attribute: String,
    /// Its value before, or `(sensitive)`, or `null`.
    pub before: String,
    /// Its value after, `(known after apply)`, or `(sensitive)`.
    pub after: String,
    /// Whether changing it forces a replacement.
    pub replaces: bool,
}

/// Whether a `*_sensitive` or `after_unknown` mask marks anything at or under it.
fn masked(mask: &Value) -> bool {
    match mask {
        Value::Bool(b) => *b,
        Value::Array(a) => a.iter().any(masked),
        Value::Object(o) => o.values().any(masked),
        _ => false,
    }
}

/// Each attribute a plan's resource changes set, from `tofu show -json` of the plan; sensitive
/// values are never shown, at any depth.
#[must_use]
pub fn plan_diff(shown: &str) -> Vec<AttrChange> {
    let v: Value = serde_json::from_str(shown).unwrap_or(Value::Null);
    let mut out = Vec::new();
    for rc in v["resource_changes"].as_array().into_iter().flatten() {
        let c = &rc["change"];
        let actions: Vec<&str> = c["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let action = match actions.as_slice() {
            ["no-op"] | [] => continue,
            ["update"] => "update",
            ["create"] => "create",
            ["delete"] => "delete",
            ["read"] => "read",
            _ => "replace",
        };
        let empty = Map::new();
        let before = c["before"].as_object().unwrap_or(&empty);
        let after = c["after"].as_object().unwrap_or(&empty);
        let replace_paths: Vec<String> = c["replace_paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.get(0).and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let b = before.get(k).unwrap_or(&Value::Null);
            let a = after.get(k).unwrap_or(&Value::Null);
            let unknown = masked(&c["after_unknown"][k.as_str()]);
            if b == a && !unknown {
                continue;
            }
            let show = |val: &Value, sensitive: &Value| {
                if masked(sensitive) {
                    "(sensitive)".to_string()
                } else {
                    val.to_string()
                }
            };
            out.push(AttrChange {
                address: rc["address"].as_str().unwrap_or_default().to_string(),
                action: action.to_string(),
                attribute: k.clone(),
                before: show(b, &c["before_sensitive"][k.as_str()]),
                after: if unknown {
                    "(known after apply)".to_string()
                } else {
                    show(a, &c["after_sensitive"][k.as_str()])
                },
                replaces: replace_paths.iter().any(|p| p == k),
            });
        }
    }
    out
}

/// A Markdown summary of `plans`: addresses, actions and changed attribute names only.
#[must_use]
pub fn summary(plans: &[UnitPlan]) -> String {
    let mut out = String::from("<!-- cobot-plan -->\n## cobot plan\n");
    for p in plans {
        let _ = write!(out, "\n### `{}`\n\n", p.unit);
        if let Some(note) = &p.note {
            let _ = write!(out, "{note}\n\n");
        }
        if p.has_changes() {
            for c in &p.changes {
                let _ = write!(out, "- {} `{}`", c.action.word(), c.address);
                if c.attributes.is_empty() {
                    out.push('\n');
                } else {
                    let _ = writeln!(out, " ({})", c.attributes.join(", "));
                }
            }
            let (add, change, destroy) = p.counts();
            let _ = write!(
                out,
                "\n{add} to add, {change} to change, {destroy} to destroy.\n"
            );
        } else {
            out.push_str("No changes.\n");
        }
    }
    out
}
