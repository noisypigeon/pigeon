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

fn files(keys: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
    keys.iter()
        .map(|(k, b)| ((*k).to_string(), b.as_bytes().to_vec()))
        .collect()
}

#[test]
fn a_sync_puts_only_changed_keys_and_deletes_stale_ones() {
    use super::publish::{TYPES, expected_of, sync_plan};
    let expected =
        expected_of(files(&[("index.html", "<html>"), ("a/index.html", "a")])).expect("keys");
    let keys: Vec<&str> = expected.keys().map(String::as_str).collect();
    assert_eq!(keys, ["a", "a/", "a/index.html", "index.html"]);
    assert!(expected_of(files(&[(TYPES, "{}")])).is_err());

    let fresh = sync_plan(&expected, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!((fresh.put.len(), fresh.unchanged), (4, 0));
    assert!(fresh.types.is_some());

    let md5 = |b: &str| crate::cobot::s3::md5_hex(b.as_bytes());
    let have: BTreeMap<String, String> = [
        ("index.html", md5("<html>")),
        ("a", md5("a")),
        ("a/", md5("old")),
        ("a/index.html", md5("a")),
        ("stale.css", md5("x")),
        (TYPES, md5("{}")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let written: BTreeMap<String, String> = expected
        .iter()
        .map(|(k, u)| (k.clone(), u.content_type.to_string()))
        .collect();
    let plan = sync_plan(&expected, &have, &written);
    assert_eq!(plan.put, ["a/"]);
    assert_eq!(plan.unchanged, 3);
    assert_eq!(plan.types, None, "the types record is current");
    assert_eq!(plan.delete, ["stale.css"]);
}

#[test]
fn probes_check_the_root_a_page_a_missing_page_and_the_www_redirect() {
    use super::checks::{Probe, probes};
    let keys = ["blog/index.html".to_string()];
    let hosts = ["example.com".to_string(), "www.example.com".to_string()];
    let list = probes(Some("example.com"), "example.com", &hosts, &keys, 7);
    assert_eq!(
        list,
        [
            Probe::Html("https://example.com/".into()),
            Probe::Ok("https://example.com/blog".into()),
            Probe::Ok("https://example.com/blog/".into()),
            Probe::Missing("https://example.com/cobot-missing-7/".into()),
            Probe::Redirect {
                from: "https://www.example.com/".into(),
                to: "https://example.com/".into(),
            },
        ]
    );
    assert!(list[0].passes(200, None, "<html>"));
    assert!(!list[0].passes(200, None, "plain"));
    assert!(list[3].passes(404, None, ""));
    assert!(list[4].passes(301, Some("https://example.com/"), ""));
    assert!(!list[4].passes(301, Some("https://www.example.com/"), ""));
    assert_eq!(probes(None, "example.com", &hosts, &keys, 7), Vec::new());
}

/// The docs gate; see the docs-under-platform ADR §2 and §3.
mod view {
    use super::super::view::{TTL, Target, cookie, gate, set_cookie, sign, target, valid};

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn a_path_names_its_key_and_nothing_leaves_the_prefix() {
        assert_eq!(target("/docs"), Some(Target::Slash));
        assert_eq!(target("/docs/"), Some(Target::Key("index.html".into())));
        assert_eq!(target("/docs/a/"), Some(Target::Key("a/index.html".into())));
        let css = Target::Key("assets/css/main.css".into());
        assert_eq!(target("/docs/assets/css/main.css"), Some(css));
        for outside in [
            "/",
            "/docsx",
            "/docs/../passkeys",
            "/docs/./a",
            "/docs/a\\b",
            "/approvals",
        ] {
            assert_eq!(target(outside), None, "{outside}");
        }
    }

    #[test]
    fn a_cookie_works_for_its_identity_until_it_expires() {
        let now = 1_000_000;
        let v = sign(KEY, "owner@example.com", now + TTL);
        assert!(valid(KEY, &v, "owner@example.com", now));
        assert!(!valid(KEY, &v, "other@example.com", now), "identity");
        assert!(!valid(KEY, &v, "owner@example.com", now + TTL), "expired");
        assert!(!valid(b"other key", &v, "owner@example.com", now), "key");
        let far = sign(KEY, "owner@example.com", now + TTL + 1);
        let (expires, tag) = v.split_once('.').expect("value");
        let moved = format!("{}.{tag}", expires.parse::<u64>().expect("n") - 1);
        // Too long-lived, an unsigned expiry, and malformed values.
        for bad in [far.as_str(), moved.as_str(), "", "x", "1.", ".abc", "n.n"] {
            assert!(!valid(KEY, bad, "owner@example.com", now), "{bad:?}");
        }
    }

    #[test]
    fn the_cookie_is_read_from_its_header_and_set_host_only() {
        let header = "a=1; __Host-pigeon-docs=123.abc; b=2";
        assert_eq!(cookie(header), Some("123.abc"));
        assert_eq!(cookie("pigeon-docs=1"), None);
        let set = set_cookie("123.abc");
        for part in [
            "__Host-pigeon-docs=123.abc",
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
        ] {
            assert!(set.contains(part), "{set}");
        }
        assert!(!set.contains("Domain"), "{set}");
    }

    #[test]
    fn the_gate_shows_no_site_and_escapes_what_it_echoes() {
        let page = gate("/docs/\"><x>", true, Some("<refused>"));
        assert!(page.contains("data-purpose=\"view:docs\""), "{page}");
        assert!(
            page.contains("&lt;refused&gt;") && !page.contains("<x>"),
            "{page}"
        );
        assert!(!page.contains("diagram"), "{page}");
        let none = gate("/docs/", false, None);
        assert!(
            none.contains("href=\"/passkeys\"") && !none.contains("data-passkey"),
            "{none}"
        );
    }
}
