//! Each site template's config contract holds together; see
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md` and
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §1.

use include_dir::Dir;
use serde_json::{Value, json};

const FILES: [&str; 2] = ["config.toml", "static/manifest.json"];

fn text(name: &str, path: &str) -> &'static str {
    site_templates::text(name, path).unwrap_or_else(|| panic!("{name}/{path} is missing"))
}

fn has(dir: &Dir<'_>, path: &str) -> bool {
    dir.get_file(path).is_some()
}

fn schema_of(name: &str) -> Value {
    serde_json::from_str(text(name, "schema.json"))
        .unwrap_or_else(|e| panic!("{name}/schema.json: {e}"))
}

fn manifest_of(name: &str) -> Value {
    serde_saphyr::from_str(text(name, "template.yaml"))
        .unwrap_or_else(|e| panic!("{name}/template.yaml: {e}"))
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
    for (name, _) in site_templates::all() {
        let schema = schema_of(name);
        let props = schema["properties"].as_object().expect("properties");
        for (key, prop) in props {
            let list = prop.get("x-site").and_then(Value::as_array);
            assert!(
                list.is_some_and(|l| !l.is_empty()),
                "{name}/schema.json: `{key}` has no x-site target"
            );
        }
    }
}

#[test]
fn every_template_names_its_theme_and_scaffold() {
    let names: Vec<&str> = site_templates::all().iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"blog") && names.contains(&"architecture"));
    for (name, dir) in site_templates::all() {
        assert!(
            text(name, "theme.toml").contains(&format!("name = \"site-template-{name}\"")),
            "{name}: theme.toml names another theme"
        );
        for f in FILES {
            assert!(
                has(dir, &format!("scaffold/{f}")),
                "{name}: scaffold/{f} is missing"
            );
        }
        let manifest = manifest_of(name);
        assert!(
            manifest["version"].is_string() && manifest["features"].is_object(),
            "{name}"
        );
    }
}

#[test]
fn the_architecture_template_has_no_content_and_only_its_rail_script() {
    assert_eq!(manifest_of("architecture")["content"], "none");
    let index = text("architecture", "templates/index.html");
    assert!(index.contains("load_data(path=\"data/architecture.json\""));
    // Its only script is the static rail.js; see the 2026-11-11 amendment of
    // .claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md.
    let rail = "<script src=\"{{ get_url(path=\"assets/js/rail.js\") }}\" defer></script>";
    assert_eq!(index.matches("<script").count(), 1, "one script");
    assert!(index.contains(rail), "the script is rail.js");
    assert!(has(
        &site_templates::ARCHITECTURE,
        "static/assets/js/rail.js"
    ));
}

/// It builds under a path, as `served_by` serves it; see
/// `.claude/decisions/2026-11-11-runtime-docs-under-platform-passkey.md` §4.
#[test]
fn the_architecture_template_links_relative_to_its_base_url() {
    let index = text("architecture", "templates/index.html");
    for root in ["href=\"/", "src=\"/"] {
        assert!(
            !index.contains(root),
            "a root-relative `{root}` in index.html"
        );
    }
    let manifest: Value =
        serde_json::from_str(text("architecture", "scaffold/static/manifest.json")).expect("json");
    assert_eq!(
        manifest["start_url"], "./",
        "start_url is relative to manifest.json"
    );
}

#[test]
fn targets_name_a_scaffold_file_and_a_key() {
    for (name, dir) in site_templates::all() {
        let mut all = Vec::new();
        targets(&schema_of(name), "", &mut all);
        assert!(all.len() > 1, "{name}/schema.json has no x-site targets");
        for (path, t) in &all {
            let file = t["file"].as_str().unwrap_or_default();
            assert!(FILES.contains(&file), "`{path}`: unknown file `{file}`");
            assert!(
                has(dir, &format!("scaffold/{file}")),
                "`{path}`: scaffold/{file} is missing"
            );
            let key = t["key"].as_str().unwrap_or_default();
            assert!(
                !key.is_empty() && key.split('.').all(|k| !k.is_empty()),
                "`{path}`: bad key `{key}`"
            );
            assert!(
                !matches!(key, "base_url" | "theme") && !key.starts_with("extra.features"),
                "`{path}`: `{key}` is the builder's or the scaffold's"
            );
        }
    }
}

#[test]
fn the_scaffold_holds_the_sections() {
    for f in ["content/posts/_index.md", "content/pages/_index.md"] {
        assert!(
            text("blog", &format!("scaffold/{f}")).contains("render = false"),
            "{f} renders; the theme has no section.html"
        );
    }
}

#[test]
fn the_blog_settings_validate() {
    let validator = jsonschema::validator_for(&schema_of("blog")).expect("schema.json compiles");
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
    assert_eq!(
        manifest_of("blog")["content"]["posts"]["filename"],
        "date-slug"
    );
}
