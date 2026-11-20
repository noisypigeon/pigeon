use serde_json::json;

use super::*;

fn cap(name: &str) -> &'static Declared {
    get(name).expect("capability")
}

#[test]
fn every_manifest_parses_and_names_its_directory() {
    for (dir, _) in SOURCES {
        let c = get(dir).expect("registered");
        assert_eq!(c.manifest().name, dir);
    }
}

#[test]
fn the_registry_is_the_members_vocabulary() {
    let mut names = names();
    names.sort_unstable();
    let mut want = crate::member::CAPABILITIES.to_vec();
    want.sort_unstable();
    assert_eq!(names, want);
    let mut grantable = grantable();
    grantable.sort_unstable();
    let mut want = crate::member::GRANTABLE.to_vec();
    want.sort_unstable();
    assert_eq!(grantable, want);
}

#[test]
fn scopes_are_checked_by_kind() {
    assert!(cap("icbc").check_scope(Some("x")).is_err());
    assert!(cap("icbc").check_scope(None).is_ok());
    assert!(cap("subdomain").check_scope(Some("example")).is_ok());
    assert!(cap("subdomain").check_scope(Some("www")).is_err());
    assert!(cap("subdomain").check_scope(None).is_err());
    assert!(cap("domain").check_scope(Some("blog")).is_ok());
    assert!(cap("domain").check_scope(Some("example.com")).is_err());
    assert!(cap("blog").check_scope(Some("-x")).is_err());
}

#[test]
fn a_grant_covers_only_its_own_scope() {
    let blog = cap("blog");
    assert!(blog.covers(Some("a"), Some("a")));
    assert!(!blog.covers(Some("a"), Some("b")));
    assert!(!blog.covers(None, Some("a")));
    assert!(cap("icbc").covers(None, None));
}

#[test]
fn config_takes_no_unknown_keys() {
    let icbc = cap("icbc");
    assert!(icbc.config(&json!({})).is_ok());
    let err = icbc.config(&json!({"x": 1})).expect_err("unknown key");
    assert!(err.0.contains('x'));
}

#[test]
fn hooks_default_to_nothing() {
    let icbc = cap("icbc");
    assert_eq!(icbc.lifecycle(&Lifecycle::Disable), Ok(Plan::default()));
    assert_eq!(icbc.behaviour(&Behaviour::OnRevoke), Outcome::Pass);
}

#[test]
fn an_operation_is_owned_by_the_capability_listing_it() {
    assert_eq!(owner_of("instances.standup"), Some(("instances", None)));
    assert_eq!(owner_of("change.status"), None);
}

/// Operation areas no capability may hand a Standard member; see the ADR's gate amendment.
const NEVER_AREAS: [&str; 12] = [
    "change.",
    "mandate.",
    "lane.",
    "grant.",
    "member.",
    "service.",
    "secrets.",
    "platform.",
    "owner.",
    "approval.",
    "deploy.",
    "infra.",
];

#[test]
fn no_capability_hands_out_an_approval_owner_credential_or_platform_operation() {
    use crate::wire::{ADMINISTRATOR_OPS, Asks, Effect, catalog};
    for c in registry() {
        let name = &c.manifest().name;
        for op in &c.manifest().operations {
            assert!(
                !NEVER_AREAS.iter().any(|a| op.starts_with(a)),
                "{name} lists `{op}`, an area no capability hands out"
            );
            assert!(
                !ADMINISTRATOR_OPS.iter().any(|(n, _)| n == op),
                "{name} lists `{op}`, which is Administrator-only"
            );
            let o = catalog()
                .iter()
                .find(|o| o.name == op)
                .unwrap_or_else(|| panic!("{name} lists `{op}`, which isn't in the catalog"));
            assert_eq!(
                o.asks(),
                Asks::None,
                "{name} lists `{op}`, which asks for approval"
            );
            assert!(
                o.effect != Effect::Apply || o.control_plane(),
                "{name} lists `{op}`, a platform apply"
            );
        }
    }
}

#[test]
fn no_operation_has_two_owners() {
    let mut seen = std::collections::BTreeMap::new();
    for c in registry() {
        for op in &c.manifest().operations {
            if let Some(first) = seen.insert(op.clone(), c.manifest().name.clone()) {
                panic!("`{op}` is listed by both {first} and {}", c.manifest().name);
            }
        }
    }
}

#[test]
fn the_registry_validates() {
    let manifests: Vec<&Manifest> = registry().iter().map(Capability::manifest).collect();
    let problems = validate::validate(&manifests, crate::wire::catalog());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn every_manifest_names_test_files_that_exist() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for c in registry() {
        for t in &c.manifest().tests {
            assert!(
                root.join(t).is_file(),
                "{} names `{t}`, which isn't a file",
                c.manifest().name
            );
        }
    }
}

#[test]
fn validate_names_each_broken_rule() {
    let blog = cap("blog").manifest().clone();
    let icbc = cap("icbc").manifest().clone();
    let says = |m: &Manifest, other: &Manifest, fragment: &str| {
        let problems = validate::validate(&[m, other], crate::wire::catalog());
        assert!(
            problems.iter().any(|p| p.contains(fragment)),
            "no problem says `{fragment}`: {problems:#?}"
        );
    };
    let with = |f: &dyn Fn(&mut Manifest)| {
        let mut m = blog.clone();
        f(&mut m);
        m
    };

    says(&with(&|m| m.version = "1.0".into()), &icbc, "x.y.z");
    says(
        &with(&|m| m.operations = vec!["nope.op".into()]),
        &icbc,
        "isn't in the catalog",
    );
    let mut a = cap("instances").manifest().clone();
    a.name = "a".into();
    let mut b = a.clone();
    b.name = "b".into();
    says(&a, &b, "is also");
    says(
        &with(&|m| {
            m.routes = vec![Route {
                door: Door::App,
                prefix: "/v0/app/learner/x/".into(),
            }];
        }),
        &icbc,
        "overlaps",
    );
    says(
        &with(&|m| {
            m.routes = vec![Route {
                door: Door::App,
                prefix: "/other/".into(),
            }];
        }),
        &icbc,
        "under its door",
    );
    says(
        &with(&|m| m.storage.object_keys = vec!["learner.index".into()]),
        &icbc,
        "is also",
    );
    says(
        &with(&|m| m.secrets = vec!["bad-name".into()]),
        &icbc,
        "Secrets Store",
    );
    says(
        &with(&|m| m.built_on = Some("zzz".into())),
        &icbc,
        "names no capability",
    );
    says(
        &with(&|m| {
            m.grantable = true;
            m.operations = vec![];
            m.routes = vec![];
        }),
        &icbc,
        "nothing to grant",
    );
    says(
        &with(&|m| m.config = json!({"type": "string"})),
        &icbc,
        "object schema",
    );
}
