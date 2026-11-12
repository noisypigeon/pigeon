//! A site's layout and assembled config from data alone, with no files.

use std::collections::BTreeMap;

use serde_json::json;

use super::assemble::configure;
use super::layout::{Listing, layout_problems, no_content_problems};

const POST: &str = "+++\ntitle = \"Hello\"\ndate = 2015-02-15\n+++\nbody\n";

fn listing() -> Listing {
    Listing {
        shown: "site".into(),
        top: Some(vec![("posts".into(), true), ("notes.txt".into(), false)]),
        nested: vec![
            ("posts/2015-02-15-hello.md".into(), false),
            ("posts/drafts".into(), true),
            ("posts/drafts/_index.md".into(), false),
        ],
        posts: vec![
            ("2015-02-15-hello.md".into(), POST.into()),
            ("hello.md".into(), POST.into()),
        ],
    }
}

#[test]
fn a_listing_alone_shows_every_layout_problem() {
    let problems = layout_problems(&listing(), true);
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems[0].contains("`notes.txt` does not belong"));
    assert!(problems[1].contains("`posts/drafts/_index.md`"));
    assert!(problems[2].starts_with("posts/hello.md: is not named"));
    assert_eq!(layout_problems(&listing(), false).len(), 2);
    let missing = Listing {
        shown: "site".into(),
        ..Listing::default()
    };
    assert_eq!(layout_problems(&missing, true), ["site is missing"]);
    assert!(no_content_problems("site", true)[0].contains("must not exist"));
    assert_eq!(no_content_problems("site", false), Vec::<String>::new());
}

#[test]
fn configure_writes_settings_theme_base_and_features_into_the_scaffold() {
    let schema = json!({ "properties": { "title": {
        "x-site": [{ "file": "config.toml", "key": "title" },
                   { "file": "static/manifest.json", "key": "name" }]
    } } });
    let features = BTreeMap::from([("search".to_string(), true)]);
    let (config, manifest) = configure(
        &schema,
        &json!({ "title": "T" }),
        "title = \"x\"\n",
        "{}",
        "site-template-blog",
        "https://example.com/",
        &features,
    )
    .expect("configures");
    let config: toml::Value = toml::from_str(&config).expect("toml");
    assert_eq!(config["title"].as_str(), Some("T"));
    assert_eq!(config["theme"].as_str(), Some("site-template-blog"));
    assert_eq!(config["base_url"].as_str(), Some("https://example.com/"));
    assert_eq!(config["extra"]["features"]["search"].as_bool(), Some(true));
    assert_eq!(manifest, "{\n  \"name\": \"T\"\n}\n");
    let bad = json!({ "properties": { "t": { "x-site": [{ "file": "x", "key": "k" }] } } });
    assert!(configure(&bad, &json!({ "t": 1 }), "", "{}", "t", "b", &features).is_err());
}
