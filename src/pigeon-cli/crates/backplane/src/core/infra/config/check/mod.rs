//! A config unit's checks; each problem is `(file, message)`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{
    BLOCK, CONFIG, CREDENTIAL, ConfigUnit, Disk, Expr, SCHEMA, SEMVER, allowed, blocks,
    declared_credentials, declared_secrets, exprs_in, parse, walk_strings, x_cobot,
};
use crate::cobot::{apex, secrets};
use crate::core::infra::providers::Allowlist;

mod blocks;

use blocks::{check_blocks, check_outputs};

/// Checks a config unit; each problem is `(file, message)`. `infra_keys` are
/// the break-glass keys a workload may never hold.
#[must_use]
pub fn validate(
    unit: &ConfigUnit,
    disk: &Disk,
    area: &str,
    allow: &Allowlist,
    infra_keys: &BTreeSet<String>,
    catalogs: &[crate::cobot::catalog::Catalog],
) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    check_top(unit, &mut problems);
    check_credentials(unit, catalogs, &mut problems);
    check_secrets(unit, disk, area, infra_keys, &mut problems);
    check_blocks(unit, allow, disk, &mut problems);
    check_outputs(unit, disk, &mut problems);
    check_apex(unit, &mut problems);
    problems
}

/// Checks `x-cobot.credentials` against the catalogs' capabilities, and each
/// `$credential` reference against it.
pub(crate) fn check_credentials(
    unit: &ConfigUnit,
    catalogs: &[crate::cobot::catalog::Catalog],
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
pub(crate) fn target_ok(t: &Value) -> bool {
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
pub(crate) fn credential_refs(unit: &ConfigUnit) -> Vec<(String, String, String)> {
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

pub(crate) fn check_apex(unit: &ConfigUnit, problems: &mut Vec<(String, String)>) {
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

pub(crate) fn check_top(unit: &ConfigUnit, problems: &mut Vec<(String, String)>) {
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
        .map(|s| crate::cobot::apex::published_values(&s.composed(&unit.config), &unit.home))
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

pub(crate) fn check_secrets(
    unit: &ConfigUnit,
    disk: &Disk,
    area: &str,
    infra_keys: &BTreeSet<String>,
    problems: &mut Vec<(String, String)>,
) {
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    let declared = declared_secrets(unit);
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
    match &disk.secrets {
        Some(Ok(values)) => {
            let present: BTreeSet<String> = values.keys().cloned().collect();
            if present != declared {
                add(
                    secrets::FILE,
                    format!("keys {present:?} differ from x-cobot.secrets {declared:?}"),
                );
            }
            for (k, v) in values {
                if !secrets::is_encrypted(v) {
                    add(secrets::FILE, format!("`{k}` is not ENC[cobot:v1:…]"));
                }
            }
        }
        Some(Err(e)) => add(secrets::FILE, e.clone()),
        None => {}
    }
}
