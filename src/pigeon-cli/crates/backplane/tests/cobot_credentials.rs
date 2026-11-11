//! Credentials requested by capability; see the phase 3 amendment of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;

const UNIT: &str = "infra/proof";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn set_unit(root: &Path, credentials: &Value, token: &str) {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": { "name": { "type": "string" } },
        "x-cobot": {
            "version": "0.1.0",
            "secrets": [],
            "credentials": credentials,
            "providers": { "cloudflare": { "api_token": token } }
        }
    });
    write(root, &format!("{UNIT}/schema.json"), &schema.to_string());
}

fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    write(
        root,
        ".config/providers.yaml",
        "description: Test allowlist.\nproviders:\n  cloudflare/cloudflare: 5.28.0\n",
    );
    write(
        root,
        "catalog/cloudflare/capabilities.yaml",
        "description: Test capabilities.\ncapabilities:\n  dns.write: { permissions: [DNS Write], scope: zone }\n  r2.write: { permissions: [Workers R2 Storage Write], scope: account }\n",
    );
    write(root, &format!("{UNIT}/config.yaml"), "name: unit\n");
    set_unit(
        root,
        &json!({ "dns": { "provider": "cloudflare", "capabilities": ["dns.write"], "scope": "zone" } }),
        "$credential.dns.api_token",
    );
    dir
}

fn problems(root: &Path) -> Vec<String> {
    let allow =
        backplane::cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    backplane::cobot::unit::validate(root, &allow)
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

#[test]
fn a_requested_credential_renders_into_one_sensitive_variable() {
    let repo = repo();
    let root = repo.path();
    assert_eq!(problems(root), Vec::<String>::new());
    let allow =
        backplane::cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    let units =
        backplane::cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    let json = backplane::cobot::render_json(
        &units[0],
        Path::new("/state"),
        &allow,
        0,
        &backplane::cobot::config::Existing::new(),
    );
    assert_eq!(
        json["provider"]["cloudflare"]["api_token"],
        "${var.credentials[\"dns\"].api_token}"
    );
    assert_eq!(json["variable"]["credentials"]["sensitive"], true);
    assert_eq!(json["variable"]["credentials"]["type"], "map(map(string))");
    assert!(json["variable"].get("secrets").is_none());
}

#[test]
fn bad_requests_and_references_fail() {
    let repo = repo();
    let root = repo.path();
    let dns = |caps: Value, scope: &str| json!({ "dns": { "provider": "cloudflare", "capabilities": caps, "scope": scope } });
    let cases: [(Value, &str, &str); 7] = [
        (
            dns(json!(["dns.nope"]), "zone"),
            "$credential.dns.api_token",
            "`dns.nope` is not in catalog/cloudflare/capabilities.yaml",
        ),
        (
            dns(json!(["r2.write"]), "zone"),
            "$credential.dns.api_token",
            "`r2.write` is minted on `account`, not `zone`",
        ),
        (
            dns(json!([]), "zone"),
            "$credential.dns.api_token",
            "non-empty `capabilities`",
        ),
        (
            json!({ "dns": { "provider": "nowhere", "capabilities": ["dns.write"], "scope": "zone" } }),
            "$credential.dns.api_token",
            "no catalog declares capabilities for `nowhere`",
        ),
        (
            json!({ "dns": { "provider": "cloudflare", "capabilities": ["dns.write"], "scope": "zone", "value": "x" } }),
            "$credential.dns.api_token",
            "takes only",
        ),
        (
            dns(json!(["dns.write"]), "zone"),
            "$credential.other.api_token",
            "`$credential.other.api_token` is not in x-cobot.credentials",
        ),
        (
            json!(["dns"]),
            "$credential.dns.api_token",
            "must map names to requests",
        ),
    ];
    for (creds, token, want) in cases {
        set_unit(root, &creds, token);
        let got = problems(root);
        assert!(
            got.iter().any(|p| p.contains(want)),
            "want `{want}` in {got:?}"
        );
    }
}

#[test]
fn the_runner_supplies_only_the_fields_a_unit_references() {
    let wanted = BTreeMap::from([("dns".to_string(), BTreeSet::from(["api_token".to_string()]))]);
    let json =
        r#"{ "dns": { "api_token": "t0ken", "extra": "n0t-needed" }, "other": { "key": 42 } }"#;
    let got = backplane::cobot::pick_credentials(Some(json), &wanted, UNIT).expect("picked");
    assert_eq!(
        got.len(),
        1,
        "another unit's malformed credential is never read"
    );
    assert_eq!(
        got["dns"].len(),
        1,
        "an unreferenced field is never forwarded"
    );
    assert_eq!(got["dns"]["api_token"], "t0ken");

    for (json, want) in [
        ("{}", "credential `dns`"),
        (r#"{ "dns": {} }"#, "`dns.api_token`"),
        (r#"{ "dns": { "api_token": 7 } }"#, "`dns.api_token`"),
        (r#"{ "dns": "t0ken" }"#, "credential `dns`"),
        ("[\"t0ken\"]", "JSON object"),
    ] {
        let err = backplane::cobot::pick_credentials(Some(json), &wanted, UNIT).expect_err(json);
        assert!(err.contains(want), "want `{want}` in {err}");
        assert!(!err.contains("t0ken"), "never echoes a value: {err}");
    }
    assert!(backplane::cobot::pick_credentials(None, &wanted, UNIT).is_err());

    let unreferenced = BTreeMap::from([("dns".to_string(), BTreeSet::new())]);
    for json in [r#"{ "dns": null }"#, r#"{ "dns": "t0ken" }"#] {
        let err =
            backplane::cobot::pick_credentials(Some(json), &unreferenced, UNIT).expect_err(json);
        assert!(
            err.contains("as an object") && !err.contains("t0ken"),
            "{err}"
        );
    }
    let empty = backplane::cobot::pick_credentials(Some(r#"{ "dns": {} }"#), &unreferenced, UNIT)
        .expect("object");
    assert!(empty["dns"].is_empty());
}

#[test]
fn the_fields_a_unit_references_are_collected() {
    let repo = repo();
    let root = repo.path();
    let units =
        backplane::cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    let c = units[0].config.as_ref().expect("config unit");
    assert_eq!(
        backplane::cobot::config::credential_fields(c),
        BTreeMap::from([("dns".to_string(), BTreeSet::from(["api_token".to_string()]))])
    );
}

#[test]
fn the_repositorys_directory_units_request_no_credentials() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let allow =
        backplane::cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    // A manifest unit's kind requests credentials by design, filled from the run's token.
    for unit in backplane::cobot::validate(&root, &root.join(".config"))
        .expect("valid")
        .into_iter()
        .filter(|u| u.path.starts_with("infra/") || u.path.starts_with("workloads/"))
    {
        let json = backplane::cobot::render_json(
            &unit,
            Path::new("/state"),
            &allow,
            0,
            &backplane::cobot::config::Existing::new(),
        );
        assert!(
            json["variable"].get("credentials").is_none(),
            "{} renders unchanged",
            unit.path
        );
    }
}

#[test]
fn an_instance_unit_apart_from_its_platform_finds_platform_modules_and_its_own_capabilities() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let platform = TempDir::new().expect("platform");
    for rel in [".config/providers.yaml", ".config/providers.lock.hcl"] {
        write(
            platform.path(),
            rel,
            &fs::read_to_string(repo_root.join(rel)).expect("read"),
        );
    }
    let module = "catalog/agnostic/modules/random-string";
    for entry in fs::read_dir(repo_root.join(module)).expect("module") {
        let name = entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        let rel = format!("{module}/{name}");
        write(
            platform.path(),
            &rel,
            &fs::read_to_string(repo_root.join(&rel)).expect("read"),
        );
    }

    let instance = TempDir::new().expect("instance");
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": { "name": { "type": "string" } },
        "x-cobot": {
            "version": "0.1.0",
            "secrets": [],
            "credentials": { "store": { "provider": "custom", "capabilities": ["custom.write"], "scope": "bucket" } },
            "singletons": { "suffix": { "module": "agnostic/random-string", "inputs": { "length": 6 } } }
        }
    });
    write(
        instance.path(),
        "infra/custom/schema.json",
        &schema.to_string(),
    );
    write(
        instance.path(),
        "infra/custom/config.yaml",
        "name: custom\n",
    );
    write(
        instance.path(),
        "infra/custom/capabilities.yaml",
        "description: An instance's own capabilities.\ncapabilities:\n  custom.write: { permissions: [Write], scope: bucket }\n",
    );

    let config = platform.path().join(".config");
    let units = backplane::cobot::validate_in(instance.path(), platform.path(), &config)
        .unwrap_or_else(|e| panic!("valid: {e}"));
    assert_eq!(
        units.iter().map(|u| u.path.as_str()).collect::<Vec<_>>(),
        ["infra/custom"]
    );
}
