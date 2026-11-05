//! The instance manifest and the operation catalog.

use std::fs;
use std::path::{Path, PathBuf};

use backplane::{Effect, Error, Instance, Request, catalog, run};
use tempfile::TempDir;

const MANIFEST: &str = "description: A test instance.\nschema: v0\nname: test\nrepository: https://example.invalid/test\npaths:\n  policies: .config/policies\n  providers: .config/providers.yaml\n  releases: .config/releases.yaml\n";

const POLICY: &str = "description: f starts with ok.\ntarget: { file: f.md }\nrules:\n  - line: { at: 1, matches: '^ok$' }\n";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn fixture(manifest: &str, f: &str) -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let config = root.path().join(".config");
    fs::create_dir_all(config.join("policies")).expect("mkdir");
    fs::write(config.join("instance.yaml"), manifest).expect("write");
    fs::write(config.join("policies/p.yaml"), POLICY).expect("write");
    fs::write(config.join("providers.yaml"), "").expect("write");
    fs::write(config.join("releases.yaml"), "").expect("write");
    fs::write(root.path().join("f.md"), f).expect("write");
    root
}

fn problems(root: &Path) -> Vec<String> {
    match Instance::load(root) {
        Err(Error::Invalid(p)) => p,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn request(operation: &str) -> Request {
    Request {
        operation: operation.into(),
        ..Request::default()
    }
}

#[test]
fn this_repository_loads() {
    let instance = Instance::load(&repo_root()).expect("valid manifest");
    assert_eq!(instance.schema, "v0");
}

#[test]
fn a_valid_manifest_loads() {
    let root = fixture(MANIFEST, "ok\n");
    assert_eq!(Instance::load(root.path()).expect("valid").name, "test");
}

#[test]
fn an_unknown_key_fails() {
    let root = fixture(&format!("{MANIFEST}extra: 1\n"), "ok\n");
    let p = problems(root.path());
    assert!(p[0].contains("extra"), "{p:?}");
}

#[test]
fn every_problem_is_reported_together() {
    let bad = MANIFEST
        .replace("schema: v0", "schema: v1")
        .replace("name: test", "name: Test")
        .replace("policies: .config/policies", "policies: /etc")
        .replace(
            "providers: .config/providers.yaml",
            "providers: ../providers.yaml",
        )
        .replace("releases: .config/releases.yaml", "releases: missing.yaml");
    let p = problems(fixture(&bad, "ok\n").path());
    assert_eq!(p.len(), 5, "{p:?}");
    for want in [
        "schema `v1`",
        "name `Test`",
        "absolute",
        "leaves the root",
        "does not exist",
    ] {
        assert!(p.iter().any(|m| m.contains(want)), "{want}: {p:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_root_fails() {
    let outside = TempDir::new().expect("tempdir");
    let root = fixture(MANIFEST, "ok\n");
    let releases = root.path().join(".config/releases.yaml");
    fs::remove_file(&releases).expect("remove");
    std::os::unix::fs::symlink(outside.path(), &releases).expect("symlink");
    let p = problems(root.path());
    assert!(
        p.iter()
            .any(|m| m.contains("paths.releases") && m.contains("leaves the root")),
        "{p:?}"
    );
}

#[test]
fn a_missing_manifest_is_io() {
    let root = TempDir::new().expect("tempdir");
    assert!(matches!(Instance::load(root.path()), Err(Error::Io { .. })));
}

#[test]
fn catalog_names_are_unique_and_dotted() {
    let mut names: Vec<_> = catalog().iter().map(|o| o.name).collect();
    for n in &names {
        let parts: Vec<_> = n.split('.').collect();
        assert_eq!(parts.len(), 2, "{n}");
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase())),
            "{n}"
        );
    }
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), catalog().len());
    let changing: Vec<(&str, Effect)> = catalog()
        .iter()
        .filter(|o| o.effect != Effect::Read)
        .map(|o| (o.name, o.effect))
        .collect();
    assert_eq!(
        changing,
        [("infra.plan", Effect::Plan), ("infra.apply", Effect::Apply)],
        "only these change anything"
    );
}

#[test]
fn policy_check_reports_violations() {
    let root = fixture(MANIFEST, "no\n");
    let instance = Instance::load(root.path()).expect("valid");
    let response = run(&instance, &request("policy.check")).expect("runs");
    assert!(!response.ok);
    assert_eq!(
        response.violations,
        ["f.md:1: p: line does not match `^ok$`"]
    );
}

#[test]
fn policy_check_holds_when_clean() {
    let root = fixture(MANIFEST, "ok\n");
    let instance = Instance::load(root.path()).expect("valid");
    let response = run(&instance, &request("policy.check")).expect("runs");
    assert!(response.ok);
    assert_eq!(response.result["ran"], 1);
}

#[test]
fn infra_validate_holds_for_this_repository() {
    let instance = Instance::load(&repo_root()).expect("valid");
    let response = run(&instance, &request("infra.validate")).expect("runs");
    assert!(response.ok, "{:?}", response.violations);
    assert!(
        response.result["units"]
            .as_array()
            .is_some_and(|u| !u.is_empty())
    );
}

#[test]
fn unknown_operations_and_arguments_are_invalid() {
    let root = fixture(MANIFEST, "ok\n");
    let instance = Instance::load(root.path()).expect("valid");
    assert!(matches!(
        run(&instance, &request("nope.nope")),
        Err(Error::Invalid(_))
    ));
    let mut with_args = request("instance.show");
    with_args.args.insert("x".into(), 1.into());
    assert!(matches!(run(&instance, &with_args), Err(Error::Invalid(_))));
}

#[test]
fn the_response_has_its_wire_shape() {
    let root = fixture(MANIFEST, "ok\n");
    let instance = Instance::load(root.path()).expect("valid");
    let value = serde_json::to_value(run(&instance, &request("instance.show")).expect("runs"))
        .expect("json");
    let mut keys: Vec<_> = value.as_object().expect("object").keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["ok", "operation", "result", "violations"]);
    assert_eq!(value["result"]["name"], "test");
}

const PR_POLICY: &str = "description: Titles start with feat.\ntarget: pull-request\nrules:\n  - field: { name: title, matches: '^feat' }\n";

fn with(operation: &str, args: serde_json::Value) -> Request {
    let serde_json::Value::Object(args) = args else {
        panic!("args are an object");
    };
    Request {
        operation: operation.into(),
        args,
    }
}

#[test]
fn policy_check_runs_the_pull_request_policy_only_when_given_its_fields() {
    let root = fixture(MANIFEST, "ok\n");
    fs::write(root.path().join(".config/policies/pr.yaml"), PR_POLICY).expect("write");
    let instance = Instance::load(root.path()).expect("valid");

    let plain = run(&instance, &request("policy.check")).expect("runs");
    assert!(plain.ok, "{:?}", plain.violations);
    assert_eq!(
        plain.result["skipped"], 1,
        "no pull request, no pull-request policy"
    );

    let good = run(
        &instance,
        &with(
            "policy.check",
            serde_json::json!({ "title": "feat: x", "body": "b", "number": 7, "files": "f.md" }),
        ),
    )
    .expect("runs");
    assert!(good.ok, "{:?}", good.violations);
    assert_eq!(good.result["skipped"], 0);

    let bad = run(
        &instance,
        &with(
            "policy.check",
            serde_json::json!({ "title": "chore: x", "body": "b", "number": "7", "files": "f.md" }),
        ),
    )
    .expect("runs");
    assert!(!bad.ok);
    assert!(
        bad.violations.iter().any(|v| v.contains("pr")),
        "{:?}",
        bad.violations
    );

    assert!(matches!(
        run(
            &instance,
            &with(
                "policy.check",
                serde_json::json!({ "body": { "not": "text" } })
            )
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn sites_build_with_no_sites_builds_nothing() {
    let root = fixture(MANIFEST, "ok\n");
    let instance = Instance::load(root.path()).expect("valid");
    let response = run(&instance, &request("sites.build")).expect("runs");
    assert!(response.ok, "{:?}", response.violations);
    assert_eq!(response.result["sites"], serde_json::json!([]));
    assert!(matches!(
        run(
            &instance,
            &with("sites.build", serde_json::json!({ "sites": "blog" }))
        ),
        Err(Error::Invalid(_))
    ));
}
