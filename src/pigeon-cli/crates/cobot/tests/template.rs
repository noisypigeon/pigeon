//! Each site template's config contract holds together; see
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md` and
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §1.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILES: [&str; 2] = ["config.toml", "static/manifest.json"];

fn templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../site-templates")
}

fn template() -> PathBuf {
    templates_dir().join("blog")
}

/// Every template: each directory in `src/site-templates/`.
fn templates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(templates_dir())
        .expect("src/site-templates")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn schema() -> Value {
    schema_of(&template())
}

fn schema_of(dir: &Path) -> Value {
    let path = dir.join("schema.json");
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
    for dir in templates() {
        let schema = schema_of(&dir);
        let props = schema["properties"].as_object().expect("properties");
        for (name, prop) in props {
            let list = prop.get("x-site").and_then(Value::as_array);
            assert!(
                list.is_some_and(|l| !l.is_empty()),
                "{}/schema.json: `{name}` has no x-site target",
                dir.display()
            );
        }
    }
}

#[test]
fn every_template_names_its_theme_and_scaffold() {
    let names: Vec<String> = templates()
        .iter()
        .map(|d| d.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"blog".to_string()) && names.contains(&"architecture".to_string()));
    for dir in templates() {
        let name = dir
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let theme = fs::read_to_string(dir.join("theme.toml")).expect("theme.toml");
        assert!(
            theme.contains(&format!("name = \"site-template-{name}\"")),
            "{name}: theme.toml names another theme"
        );
        for f in ["config.toml", "static/manifest.json"] {
            assert!(
                dir.join("scaffold").join(f).is_file(),
                "{name}: scaffold/{f} is missing"
            );
        }
        let manifest: Value = serde_saphyr::from_str(
            &fs::read_to_string(dir.join("template.yaml")).expect("template.yaml"),
        )
        .expect("template.yaml");
        assert!(
            manifest["version"].is_string() && manifest["features"].is_object(),
            "{name}"
        );
    }
}

#[test]
fn the_architecture_template_has_no_content_and_only_its_rail_script() {
    let dir = templates_dir().join("architecture");
    let manifest: Value = serde_saphyr::from_str(
        &fs::read_to_string(dir.join("template.yaml")).expect("template.yaml"),
    )
    .expect("template.yaml");
    assert_eq!(manifest["content"], "none");
    let index = fs::read_to_string(dir.join("templates/index.html")).expect("index.html");
    assert!(index.contains("load_data(path=\"data/architecture.json\""));
    // Its only script is the static rail.js; see the 2026-11-11 amendment of
    // .claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md.
    let rail = "<script src=\"/assets/js/rail.js\" defer></script>";
    assert_eq!(index.matches("<script").count(), 1, "one script");
    assert!(index.contains(rail), "the script is rail.js");
    assert!(dir.join("static/assets/js/rail.js").is_file());
}

#[test]
fn targets_name_a_scaffold_file_and_a_key() {
    for dir in templates() {
        targets_hold(&dir);
    }
}

fn targets_hold(dir: &Path) {
    let mut all = Vec::new();
    targets(&schema_of(dir), "", &mut all);
    assert!(
        all.len() > 1,
        "{}/schema.json has no x-site targets",
        dir.display()
    );
    for (path, t) in &all {
        let file = t["file"].as_str().unwrap_or_default();
        assert!(FILES.contains(&file), "`{path}`: unknown file `{file}`");
        assert!(
            dir.join("scaffold").join(file).is_file(),
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
