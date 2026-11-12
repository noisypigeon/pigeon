//! A site's assembled Zola project: its settings checked against the tag's schema and written
//! into the scaffold's `config.toml` and `manifest.json`, from texts the shell read; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md`.

use std::collections::BTreeMap;

use serde_json::Value;

/// `settings` against the tag's schema, with every error.
///
/// # Errors
///
/// Each way `settings` breaks `schema`, joined.
pub fn check_settings(schema: &Value, settings: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema).map_err(|e| format!("schema.json: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(settings)
        .map(|e| {
            let at = e.instance_path().to_string();
            format!("static_site.site{}: {e}", at.replace('/', "."))
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Each `(file, key, value)` the settings write: every set property whose
/// schema carries `x-site`, at any depth.
pub fn site_targets(schema: &Value, value: &Value, out: &mut Vec<(String, String, Value)>) {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, prop) in props {
        let Some(v) = value.get(name) else { continue };
        for t in prop
            .get("x-site")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(file), Some(key)) = (t["file"].as_str(), t["key"].as_str()) {
                out.push((file.to_string(), key.to_string(), v.clone()));
            }
        }
        site_targets(prop, v, out);
    }
}

/// Sets the dotted `key` in a TOML table, creating tables on the way.
///
/// # Errors
///
/// When `key` crosses a non-table.
pub fn set_toml(doc: &mut toml::Value, key: &str, value: toml::Value) -> Result<(), String> {
    let mut at = doc;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let table = at
            .as_table_mut()
            .ok_or_else(|| format!("config.toml: `{key}` crosses a non-table"))?;
        if i + 1 == parts.len() {
            table.insert((*part).to_string(), value);
            return Ok(());
        }
        at = table
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    }
    Ok(())
}

/// Sets the dotted `key` in a JSON object, creating objects on the way.
///
/// # Errors
///
/// When `key` crosses a non-object.
pub fn set_json(doc: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let mut at = doc;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let map = at
            .as_object_mut()
            .ok_or_else(|| format!("manifest.json: `{key}` crosses a non-object"))?;
        if i + 1 == parts.len() {
            map.insert((*part).to_string(), value);
            return Ok(());
        }
        at = map
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    Ok(())
}

/// The scaffold's `config.toml` and `static/manifest.json` texts with `settings` written in
/// where `schema`'s `x-site` says, and the theme, base URL and feature flags set; the texts to
/// write back.
///
/// # Errors
///
/// What is wrong, as the build reports it.
pub fn configure(
    schema: &Value,
    settings: &Value,
    config_text: &str,
    manifest_text: &str,
    theme: &str,
    base: &str,
    features: &BTreeMap<String, bool>,
) -> Result<(String, String), String> {
    let mut config: toml::Value =
        toml::from_str(config_text).map_err(|e| format!("scaffold/config.toml: {e}"))?;
    let mut manifest: Value = serde_json::from_str(manifest_text)
        .map_err(|e| format!("scaffold/static/manifest.json: {e}"))?;
    let mut targets = Vec::new();
    site_targets(schema, settings, &mut targets);
    for (file, key, value) in targets {
        match file.as_str() {
            "config.toml" => {
                let v = toml::Value::try_from(&value).map_err(|e| format!("{key}: {e}"))?;
                set_toml(&mut config, &key, v)?;
            }
            "static/manifest.json" => set_json(&mut manifest, &key, value)?,
            other => return Err(format!("x-site: unknown file `{other}`")),
        }
    }
    set_toml(&mut config, "theme", toml::Value::String(theme.to_string()))?;
    set_toml(
        &mut config,
        "base_url",
        toml::Value::String(base.to_string()),
    )?;
    let flags = features
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::Boolean(*v)))
        .collect();
    set_toml(&mut config, "extra.features", toml::Value::Table(flags))?;
    let config = toml::to_string(&config).map_err(|e| format!("config.toml: {e}"))?;
    let mut json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("manifest.json: {e}"))?;
    json.push('\n');
    Ok((config, json))
}
