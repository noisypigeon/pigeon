//! Catalogs under `catalog/` with an `infra/` fallback, and their capabilities; see
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md` §2–§3.

use std::fmt::Write as _;
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

fn module(root: &Path, dir: &str, vars: &[&str], extra: &str) {
    let mut tf = String::new();
    for v in vars {
        let _ = write!(
            tf,
            "variable \"{v}\" {{\n  type    = any\n  default = null\n}}\n"
        );
    }
    tf.push_str("resource \"terraform_data\" \"this\" {\n  input = null\n}\n\n");
    tf.push_str("output \"result\" {\n  value = terraform_data.this.id\n}\n");
    tf.push_str(extra);
    write(root, &format!("{dir}/main.tf"), &tf);
    write(
        root,
        &format!("{dir}/module.yaml"),
        "version: 0.1.0\ndescription: A test module.\n",
    );
}

fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["items"],
        "x-cobot": {
            "version": "0.1.0",
            "each": { "suffixes": { "module": "agnostic/suffix", "over": "items", "for_each": "$item.name", "inputs": { "length": 6 } } }
        },
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": { "name": { "type": "string" } }
                },
                "x-cobot": { "module": "item", "for_each": "$item.name", "inputs": { "name": "$item.name" } }
            }
        }
    })
}

/// `agnostic` and `proof` catalogs in `area` (`infra` or `catalog`); the units stay in `infra/`.
fn repo(area: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    write(root, "infra/agnostic/config.yaml", "{}\n");
    write(
        root,
        &format!("{area}/agnostic/schema.json"),
        r#"{ "type": "object", "additionalProperties": false, "x-cobot": { "version": "0.1.0", "secrets": [], "providers": {} } }"#,
    );
    module(
        root,
        &format!("{area}/agnostic/modules/suffix"),
        &["length"],
        "",
    );
    write(
        root,
        &format!("{UNIT}/config.yaml"),
        "items: [{ name: a }]\n",
    );
    write(
        root,
        &format!("{area}/proof/schema.json"),
        &serde_json::to_string_pretty(&schema()).expect("json"),
    );
    module(root, &format!("{area}/proof/modules/item"), &["name"], "");
    dir
}

fn problems(root: &Path) -> Vec<String> {
    backplane::cobot::unit::validate(root, &backplane::cobot::providers::Allowlist::default())
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

fn sources(root: &Path) -> (String, String) {
    let units =
        backplane::cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    let json = backplane::cobot::render_json(
        &units[0],
        Path::new("/state"),
        &backplane::cobot::providers::Allowlist::default(),
        0,
        &backplane::cobot::config::Existing::new(),
    );
    let m = &json["module"];
    let get = |k: &str| m[k]["source"].as_str().expect("source").to_string();
    (get("suffixes"), get("items"))
}

#[test]
fn infra_catalogs_still_resolve() {
    let repo = repo("infra");
    let root = repo.path();
    assert_eq!(problems(root), Vec::<String>::new());
    let (agnostic, own) = sources(root);
    assert!(
        Path::new(&agnostic).ends_with("infra/agnostic/modules/suffix"),
        "{agnostic}"
    );
    assert!(
        Path::new(&own).ends_with("infra/proof/modules/item"),
        "{own}"
    );
}

#[test]
fn moved_catalogs_resolve_from_catalog() {
    let repo = repo("catalog");
    let root = repo.path();
    assert_eq!(problems(root), Vec::<String>::new());
    let (agnostic, own) = sources(root);
    assert!(
        Path::new(&agnostic).ends_with("catalog/agnostic/modules/suffix"),
        "{agnostic}"
    );
    assert!(
        Path::new(&own).ends_with("catalog/proof/modules/item"),
        "{own}"
    );
    let units = backplane::cobot::validate(root, &root.join(".config")).expect("valid");
    let proof = units.iter().find(|u| u.path == UNIT).expect("unit");
    assert!(proof.dir.ends_with(UNIT), "units stay in infra/");
}

#[test]
fn agnostic_falls_back_across_areas() {
    let repo = repo("infra");
    let root = repo.path();
    fs::create_dir_all(root.join("catalog/proof")).expect("mkdir");
    for f in ["schema.json", "modules"] {
        fs::rename(
            root.join("infra/proof").join(f),
            root.join("catalog/proof").join(f),
        )
        .expect("mv");
    }
    assert_eq!(problems(root), Vec::<String>::new());
    let (agnostic, own) = sources(root);
    assert!(
        Path::new(&agnostic).ends_with("infra/agnostic/modules/suffix"),
        "{agnostic}"
    );
    assert!(
        Path::new(&own).ends_with("catalog/proof/modules/item"),
        "{own}"
    );
}

#[test]
fn catalog_modules_are_scanned() {
    let repo = repo("catalog");
    let root = repo.path();
    module(
        root,
        "catalog/proof/modules/item",
        &["name"],
        "resource \"terraform_data\" \"x\" {\n  provisioner \"local-exec\" {\n    command = \"true\"\n  }\n}\n",
    );
    let got = problems(root);
    assert!(
        got.iter()
            .any(|p| p.starts_with("catalog/proof/modules/item/main.tf")
                && p.contains("provisioners")),
        "{got:?}"
    );
}

const CAPS: &str = "description: Test capabilities.\ncapabilities:\n  dns.write: { permissions: [DNS Write], scope: zone }\n  r2.bucket.write: { permissions: [R2 Write], scope: bucket }\n";

#[test]
fn capabilities_load_from_catalog_or_infra() {
    let repo = repo("catalog");
    let root = repo.path();
    write(root, "catalog/proof/capabilities.yaml", CAPS);
    write(root, "infra/legacy/capabilities.yaml", CAPS);
    let (catalogs, problems_found) = backplane::cobot::catalog::capabilities(root);
    assert_eq!(problems_found, Vec::new());
    let found: Vec<(&str, &str)> = catalogs
        .iter()
        .map(|c| (c.provider.as_str(), c.path.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("legacy", "infra/legacy/capabilities.yaml"),
            ("proof", "catalog/proof/capabilities.yaml")
        ]
    );
    let proof = &catalogs[1].capabilities["dns.write"];
    assert_eq!(proof.permissions, ["DNS Write"]);
    assert_eq!(proof.scope, "zone");
    assert_eq!(problems(root), Vec::<String>::new());
}

#[test]
fn invalid_capabilities_fail_validation() {
    for (content, want) in [
        (
            "description: x.\ncapabilities:\n  Write: { permissions: [a], scope: zone }\n",
            "not a dotted lowercase name",
        ),
        (
            "description: x.\ncapabilities:\n  dns.write: { permissions: [], scope: zone }\n",
            "names no permissions",
        ),
        (
            "description: x.\ncapabilities:\n  dns.write: { permissions: [a, a], scope: zone }\n",
            "repeats permission",
        ),
        (
            "description: x.\ncapabilities:\n  dns.write: { permissions: [a], scope: Zone }\n",
            "not lowercase-dash",
        ),
        (
            "description: x.\ncapabilities:\n  dns.write: { permissions: [a], scope: zone, extra: 1 }\n",
            "extra",
        ),
        (
            "description: x.\ncapabilities: {}\n",
            "declares no capabilities",
        ),
        (
            "description: ' '\ncapabilities:\n  dns.write: { permissions: [a], scope: zone }\n",
            "`description` is empty",
        ),
    ] {
        let repo = repo("catalog");
        let root = repo.path();
        write(root, "catalog/proof/capabilities.yaml", content);
        let got = problems(root);
        assert!(
            got.iter()
                .any(|p| p.starts_with("catalog/proof/capabilities.yaml") && p.contains(want)),
            "want `{want}` in {got:?}"
        );
    }
}

#[test]
fn home_prefers_a_catalog_with_a_schema() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    assert_eq!(
        backplane::cobot::catalog::home(root, "cloudflare"),
        root.join("infra/cloudflare")
    );
    fs::create_dir_all(root.join("catalog/cloudflare")).expect("mkdir");
    assert_eq!(
        backplane::cobot::catalog::home(root, "cloudflare"),
        root.join("infra/cloudflare"),
        "an empty catalog directory is not a catalog"
    );
    write(root, "catalog/cloudflare/schema.json", "{}");
    assert_eq!(
        backplane::cobot::catalog::home(root, "cloudflare"),
        root.join("catalog/cloudflare")
    );
    assert_eq!(
        backplane::cobot::catalog::for_unit(root, "infra/cloudflare"),
        Some(root.join("catalog/cloudflare"))
    );
    assert_eq!(
        backplane::cobot::catalog::for_unit(root, "infra/cloudflare/x"),
        None
    );
    assert_eq!(
        backplane::cobot::catalog::for_unit(root, "workloads/cloudflare"),
        None
    );
}

#[test]
fn a_local_schema_wins_over_its_catalog_copy() {
    let repo = repo("infra");
    let root = repo.path();
    for f in ["proof/schema.json", "agnostic/schema.json"] {
        write(
            root,
            &format!("catalog/{f}"),
            &fs::read_to_string(root.join("infra").join(f)).expect("read"),
        );
    }
    module(root, "catalog/proof/modules/item", &["name"], "");
    let (units, _) = backplane::cobot::unit::discover(root);
    let unit = units.iter().find(|u| u.path == UNIT).expect("unit");
    let json = backplane::cobot::render_json(
        unit,
        Path::new("/state"),
        &backplane::cobot::providers::Allowlist::default(),
        0,
        &backplane::cobot::config::Existing::new(),
    );
    let get = |k: &str| {
        json["module"][k]["source"]
            .as_str()
            .expect("source")
            .to_string()
    };
    let (agnostic, own) = (get("suffixes"), get("items"));
    assert!(
        Path::new(&agnostic).ends_with("infra/agnostic/modules/suffix"),
        "{agnostic}"
    );
    assert!(
        Path::new(&own).ends_with("infra/proof/modules/item"),
        "{own}"
    );
    assert_eq!(backplane::cobot::catalog::for_unit(root, UNIT), None);
    write(root, "infra/cloudflare/schema.json", "{}");
    write(root, "catalog/cloudflare/schema.json", "{}");
    assert_eq!(
        backplane::cobot::catalog::home(root, "cloudflare"),
        root.join("infra/cloudflare")
    );
    write(root, "infra/cloudflare/capabilities.yaml", CAPS);
    write(
        root,
        "catalog/cloudflare/capabilities.yaml",
        "description: Other.\ncapabilities:\n  zone.read: { permissions: [Zone Read], scope: zone }\n",
    );
    let (catalogs, found) = backplane::cobot::catalog::capabilities(root);
    let cf = catalogs
        .iter()
        .find(|c| c.provider == "cloudflare")
        .expect("cloudflare");
    assert_eq!(
        cf.path, "infra/cloudflare/capabilities.yaml",
        "follows home()"
    );
    assert!(
        found
            .iter()
            .any(|p| p.path == "catalog/cloudflare/schema.json"),
        "capabilities reports duplicates: {found:?}"
    );
    let got = problems(root);
    for p in [
        "catalog/proof/schema.json",
        "catalog/agnostic/schema.json",
        "catalog/cloudflare/schema.json",
    ] {
        assert!(
            got.iter()
                .any(|g| g.starts_with(p) && g.contains("a catalog lives in one place")),
            "want `{p}` in {got:?}"
        );
    }
}
