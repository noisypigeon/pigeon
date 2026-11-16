//! Each template's `template.yaml` holds its declared shape; see
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md`.

use serde_json::Value;

fn manifest(name: &str) -> Value {
    let text = site_templates::text(name, "template.yaml")
        .unwrap_or_else(|| panic!("{name}: template.yaml"));
    serde_saphyr::from_str(text).unwrap_or_else(|e| panic!("{name}/template.yaml: {e}"))
}

fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn every_template_yaml_is_valid() {
    for (name, _) in site_templates::all() {
        let m = manifest(name);
        let keys = [
            "version",
            "description",
            "license",
            "zola",
            "features",
            "content",
        ];
        for k in m.as_object().expect("a mapping").keys() {
            assert!(keys.contains(&k.as_str()), "{name}: unknown key `{k}`");
        }
        assert!(
            m["version"].as_str().is_some_and(is_semver),
            "{name}: version is not X.Y.Z"
        );
        assert!(
            m["zola"].as_str().is_some_and(is_semver),
            "{name}: zola is not X.Y.Z"
        );
        for k in ["description", "license"] {
            assert!(
                m[k].as_str().is_some_and(|s| !s.trim().is_empty()),
                "{name}: `{k}` is empty"
            );
        }
        for (f, decl) in m["features"].as_object().expect("features") {
            assert!(decl["default"].is_boolean(), "{name}: `{f}` has no default");
            assert!(
                decl["description"].as_str().is_some_and(|s| !s.is_empty()),
                "{name}: `{f}` has no description"
            );
            assert!(!f.starts_with("no-"), "{name}: `{f}` reads as a negation");
        }
        let content = &m["content"];
        assert!(
            content == "none" || content.get("posts").is_some_and(Value::is_object),
            "{name}: content is neither `none` nor `posts`"
        );
    }
}

#[test]
fn the_crate_embeds_every_template_and_nothing_else() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("crate dir")
        .flatten()
        .filter(|e| e.path().join("template.yaml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let embedded: Vec<&str> = site_templates::all().iter().map(|(n, _)| *n).collect();
    assert_eq!(on_disk, embedded);
    assert!(site_templates::get("blog").is_some() && site_templates::get("nope").is_none());
}
