//! The static-site template's config contract holds together; see
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILES: [&str; 2] = ["config.toml", "static/manifest.json"];

fn template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../site-templates/blog")
}

fn schema() -> Value {
    let path = template().join("schema.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every `x-site` target in `schema`, with the property path that carries it.
fn targets(schema: &Value, at: &str, out: &mut Vec<(String, Value)>) {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, prop) in props {
        let path = if at.is_empty() {
            name.clone()
        } else {
            format!("{at}.{name}")
        };
        for t in prop
            .get("x-site")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.push((path.clone(), t.clone()));
        }
        targets(prop, &path, out);
    }
}

#[test]
fn every_setting_has_a_target() {
    let schema = schema();
    let props = schema["properties"].as_object().expect("properties");
    for (name, prop) in props {
        let list = prop.get("x-site").and_then(Value::as_array);
        assert!(
            list.is_some_and(|l| !l.is_empty()),
            "schema.json: `{name}` has no x-site target"
        );
    }
}

#[test]
fn targets_name_a_scaffold_file_and_a_key() {
    let mut all = Vec::new();
    targets(&schema(), "", &mut all);
    assert!(all.len() > 1, "schema.json has no x-site targets");
    for (path, t) in &all {
        let file = t["file"].as_str().unwrap_or_default();
        assert!(FILES.contains(&file), "`{path}`: unknown file `{file}`");
        assert!(
            template().join("scaffold").join(file).is_file(),
            "`{path}`: scaffold/{file} is missing"
        );
        let key = t["key"].as_str().unwrap_or_default();
        assert!(
            !key.is_empty() && key.split('.').all(|k| !k.is_empty()),
            "`{path}`: bad key `{key}`"
        );
        assert!(
            !matches!(key, "base_url" | "theme") && !key.starts_with("extra.features"),
            "`{path}`: `{key}` is cobot's or the scaffold's"
        );
    }
}

#[test]
fn the_scaffold_holds_the_sections() {
    for f in ["content/posts/_index.md", "content/pages/_index.md"] {
        let path = template().join("scaffold").join(f);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            text.contains("render = false"),
            "{f} renders; the theme has no section.html"
        );
    }
}

#[test]
fn the_blog_settings_validate() {
    let validator = jsonschema::validator_for(&schema()).expect("schema.json compiles");
    let blog = json!({
        "title": "Willow Graysen",
        "short_name": "noisypigeon",
        "description": "Infra dev, data hoarder, mechanic-ish, photographer.",
        "intro": "Hi.",
        "avatar": "/assets/images/site/avatar.png",
        "social_preview": "/assets/images/site/social_preview.png",
        "author": { "alternate_names": ["Willow Finch"], "page": "/pages/about" },
        "nav": [
            { "title": "About", "link": "/pages/about" },
            { "title": "GitHub", "link": "https://github.com/noisypigeon" }
        ],
        "icons": [{ "src": "/assets/images/site/icon-192.png", "sizes": "192x192", "type": "image/png" }]
    });
    assert!(validator.is_valid(&blog));
    for bad in [
        json!({ "description": "no title" }),
        json!({ "title": "t", "description": "d", "nav": [{ "title": "x", "link": "ftp://x" }] }),
        json!({ "title": "t", "description": "d", "theme_color": { "light": "#fff" } }),
        json!({ "title": "t", "description": "d", "typo": true }),
    ] {
        assert!(!validator.is_valid(&bad), "accepted {bad}");
    }
}

#[test]
fn posts_are_named_by_date() {
    let path = template().join("template.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let manifest: Value = serde_saphyr::from_str(&text).expect("template.yaml");
    assert_eq!(manifest["content"]["posts"]["filename"], "date-slug");
}
