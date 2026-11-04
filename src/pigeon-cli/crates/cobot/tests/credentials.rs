//! Credentials requested by capability; see the phase 3 amendment of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::collections::BTreeSet;
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
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    cobot::unit::validate(root, &allow)
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

#[test]
fn a_requested_credential_renders_into_one_sensitive_variable() {
    let repo = repo();
    let root = repo.path();
    assert_eq!(problems(root), Vec::<String>::new());
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    let units = cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    let json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &allow,
        0,
        &cobot::config::Existing::new(),
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
fn the_runner_supplies_only_what_a_unit_requests() {
    let wanted = BTreeSet::from(["dns".to_string()]);
    let json = r#"{ "dns": { "api_token": "t0ken" }, "other": { "api_token": "n0pe" } }"#;
    let got = cobot::pick_credentials(Some(json), &wanted, UNIT).expect("picked");
    assert_eq!(got.len(), 1);
    assert_eq!(got["dns"]["api_token"], "t0ken");

    let missing = cobot::pick_credentials(Some("{}"), &wanted, UNIT).expect_err("missing");
    assert!(missing.contains("credential `dns`"), "{missing}");
    let bad = cobot::pick_credentials(Some("{ \"dns\": \"t0ken\" }"), &wanted, UNIT)
        .expect_err("bad shape");
    assert!(!bad.contains("t0ken"), "never echoes a value: {bad}");
    assert!(cobot::pick_credentials(None, &wanted, UNIT).is_err());
}

#[test]
fn the_repositorys_units_request_no_credentials() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    for unit in cobot::validate(&root, &root.join(".config")).expect("valid") {
        let json = cobot::render_json(
            &unit,
            Path::new("/state"),
            &allow,
            0,
            &cobot::config::Existing::new(),
        );
        assert!(
            json["variable"].get("credentials").is_none(),
            "{} renders unchanged",
            unit.path
        );
    }
}
