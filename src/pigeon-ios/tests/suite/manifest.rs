//! `project.yml`: its targets, and the `Info.plist` keys and entitlements it generates; see
//! .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md §1.

use serde_json::Value;

fn project() -> Value {
    let text =
        std::fs::read_to_string(pigeon_ios::root().join("project.yml")).expect("project.yml");
    serde_saphyr::from_str(&text).expect("project.yml parses")
}

fn target<'a>(p: &'a Value, name: &str) -> &'a Value {
    let t = &p["targets"][name];
    assert!(t.is_object(), "project.yml has no target {name}");
    t
}

/// A build setting as text; YAML may read `YES` and `NO` as booleans.
fn setting(t: &Value, key: &str) -> Option<String> {
    match &t["settings"]["base"][key] {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(if *b { "YES" } else { "NO" }.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[test]
fn the_three_targets_build_for_ios_from_their_own_directories() {
    let p = project();
    for (name, kind) in [
        ("PigeonCore", "framework.static"),
        ("PigeonCoreTests", "bundle.unit-test"),
        ("Pigeon", "application"),
    ] {
        let t = target(&p, name);
        assert_eq!(t["type"], kind, "{name}'s type");
        assert_eq!(t["platform"], "iOS", "{name}'s platform");
        assert_eq!(t["sources"], serde_json::json!([name]), "{name}'s sources");
        assert!(pigeon_ios::root().join(name).is_dir(), "{name}/ is missing");
        assert!(
            pigeon_ios::SWIFT_DIRS.contains(&name),
            "{name} isn't in SWIFT_DIRS"
        );
    }
    assert_eq!(
        p["targets"].as_object().expect("targets").len(),
        pigeon_ios::SWIFT_DIRS.len(),
        "a target isn't in SWIFT_DIRS"
    );
    assert!(
        p["options"]["deploymentTarget"]["iOS"].is_string(),
        "no iOS deployment target"
    );
}

#[test]
fn the_app_and_its_tests_link_pigeon_core_and_the_scheme_runs_them() {
    let p = project();
    let core = serde_json::json!([{ "target": "PigeonCore" }]);
    assert_eq!(target(&p, "Pigeon")["dependencies"], core);
    assert_eq!(target(&p, "PigeonCoreTests")["dependencies"], core);
    assert_eq!(
        target(&p, "Pigeon")["scheme"]["testTargets"],
        serde_json::json!(["PigeonCoreTests"])
    );
}

#[test]
fn every_target_generates_its_info_plist_under_the_apps_bundle_id() {
    let p = project();
    let app =
        setting(target(&p, "Pigeon"), "PRODUCT_BUNDLE_IDENTIFIER").expect("the app's bundle id");
    assert_eq!(app, "dev.pigeon.app");
    for name in pigeon_ios::SWIFT_DIRS {
        let t = target(&p, name);
        assert_eq!(
            setting(t, "GENERATE_INFOPLIST_FILE").as_deref(),
            Some("YES"),
            "{name}"
        );
        let id = setting(t, "PRODUCT_BUNDLE_IDENTIFIER").expect("bundle id");
        assert!(
            id == app || id.starts_with(&format!("{app}.")),
            "{name}'s bundle id {id}"
        );
    }
}

#[test]
fn the_apps_info_plist_has_its_required_keys() {
    let p = project();
    let app = target(&p, "Pigeon");
    // TestFlight asks no export-compliance question (the iOS ADR §1).
    assert_eq!(
        setting(app, "INFOPLIST_KEY_ITSAppUsesNonExemptEncryption").as_deref(),
        Some("NO")
    );
    assert_eq!(
        setting(app, "INFOPLIST_KEY_CFBundleDisplayName").as_deref(),
        Some("Pigeon")
    );
    for key in [
        "INFOPLIST_KEY_UILaunchScreen_Generation",
        "INFOPLIST_KEY_UIApplicationSceneManifest_Generation",
    ] {
        assert_eq!(setting(app, key).as_deref(), Some("YES"), "{key}");
    }
    for key in [
        "INFOPLIST_KEY_UISupportedInterfaceOrientations",
        "INFOPLIST_KEY_UISupportedInterfaceOrientations_iPad",
        "MARKETING_VERSION",
        "CURRENT_PROJECT_VERSION",
    ] {
        let v = setting(app, key).or_else(|| match &p["settings"]["base"][key] {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });
        assert!(v.is_some_and(|v| !v.trim().is_empty()), "{key} is unset");
    }
    assert_eq!(app["info"]["path"], "Pigeon/Info.plist");
    let schemes = &app["info"]["properties"]["CFBundleURLTypes"][0]["CFBundleURLSchemes"];
    assert_eq!(
        *schemes,
        serde_json::json!(["pigeon"]),
        "the pigeon: URL scheme"
    );
}

/// Each privacy API, and the `Info.plist` key that must describe its use.
const PRIVACY: [(&str, &str); 1] = [("AVCaptureDevice", "INFOPLIST_KEY_NSCameraUsageDescription")];

#[test]
fn a_privacy_api_the_app_uses_has_its_usage_description() {
    let p = project();
    let app = target(&p, "Pigeon");
    let mut swift = String::new();
    for dir in pigeon_ios::SWIFT_DIRS {
        for f in pigeon_ios::swift_files(&pigeon_ios::root().join(dir)).expect("swift files") {
            swift += &std::fs::read_to_string(f).expect("read");
        }
    }
    for (api, key) in PRIVACY {
        if swift.contains(api) {
            assert!(
                setting(app, key).is_some_and(|v| !v.trim().is_empty()),
                "the Swift uses {api}, so {key} must be set"
            );
        }
    }
}

#[test]
fn passkeys_are_bound_to_the_host_the_app_calls() {
    let p = project();
    let app = target(&p, "Pigeon");
    let api =
        std::fs::read_to_string(pigeon_ios::root().join("Pigeon/API.swift")).expect("API.swift");
    let host = api
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("static let host = \"")?
                .strip_suffix('"')
        })
        .expect("API.swift's host");
    let domains = &app["entitlements"]["properties"]["com.apple.developer.associated-domains"];
    assert_eq!(
        *domains,
        serde_json::json!([format!("webcredentials:{host}")])
    );
    assert_eq!(app["entitlements"]["path"], "Pigeon/Pigeon.entitlements");
}
