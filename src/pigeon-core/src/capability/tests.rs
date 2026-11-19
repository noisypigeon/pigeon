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
