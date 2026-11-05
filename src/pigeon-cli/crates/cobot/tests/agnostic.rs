//! Agnostic module references and `x-cobot.each` blocks; see
//! `.claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md`.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

const STATE_PASS: &str = "state-passphrase-0123456789";
const UNIT: &str = "infra/proof";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

/// A one-resource module of built-ins, with `extra` appended to its `main.tf`.
fn module(root: &Path, dir: &str, vars: &[&str], extra: &str) {
    let mut tf = String::new();
    for v in vars {
        let _ = write!(
            tf,
            "variable \"{v}\" {{\n  type    = any\n  default = null\n}}\n"
        );
    }
    tf.push_str("resource \"terraform_data\" \"this\" {\n  input = null\n}\n\n");
    tf.push_str("output \"id\" {\n  value = terraform_data.this.id\n}\n\n");
    tf.push_str("output \"result\" {\n  value = terraform_data.this.id\n}\n");
    tf.push_str(extra);
    write(root, &format!("{dir}/main.tf"), &tf);
    write(
        root,
        &format!("{dir}/module.yaml"),
        "version: 0.1.0\ndescription: A test module.\n",
    );
}

fn schema(each: &Value) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["label", "items"],
        "x-cobot": { "version": "0.1.0", "each": each },
        "properties": {
            "label": { "type": "string" },
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": { "name": { "type": "string" } }
                },
                "x-cobot": {
                    "module": "item",
                    "for_each": "$item.name",
                    "inputs": { "name": "$item.name", "suffix": "$module.suffixes[$item.name].result" }
                }
            }
        }
    })
}

fn suffixes() -> Value {
    json!({ "suffixes": { "module": "agnostic/suffix", "over": "items", "for_each": "$item.name", "inputs": { "length": 6 } } })
}

fn set_each(root: &Path, each: &Value) {
    write(
        root,
        &format!("{UNIT}/schema.json"),
        &serde_json::to_string_pretty(&schema(each)).expect("json"),
    );
}

fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    write(root, "infra/agnostic/config.yaml", "{}\n");
    write(
        root,
        "infra/agnostic/schema.json",
        r#"{ "type": "object", "additionalProperties": false, "x-cobot": { "version": "0.1.0", "secrets": [], "providers": {} } }"#,
    );
    module(root, "infra/agnostic/modules/suffix", &["length"], "");
    write(
        root,
        &format!("{UNIT}/config.yaml"),
        "label: proof\nitems: [{ name: a }, { name: b }]\n",
    );
    set_each(root, &suffixes());
    module(
        root,
        &format!("{UNIT}/modules/item"),
        &["name", "suffix"],
        "",
    );
    dir
}

fn problems(root: &Path) -> Vec<String> {
    cobot::unit::validate(root, &cobot::providers::Allowlist::default())
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

fn assert_fails(root: &Path, want: &str) {
    let got = problems(root);
    assert!(
        got.iter().any(|p| p.contains(want)),
        "want `{want}` in {got:?}"
    );
}

fn render(root: &Path) -> Value {
    let units = cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    cobot::render_json(
        &units[0],
        Path::new("/state"),
        &cobot::providers::Allowlist::default(),
        0,
        &cobot::config::Existing::new(),
    )
}

#[test]
fn agnostic_modules_and_each_blocks_render() {
    let repo = repo();
    let root = repo.path();
    assert_eq!(problems(root), Vec::<String>::new());
    let json = render(root);
    let m = &json["module"];
    let source = m["suffixes"]["source"].as_str().expect("source");
    assert!(
        Path::new(source).ends_with("infra/agnostic/modules/suffix"),
        "{source}"
    );
    let keys: Vec<&String> = m["suffixes"]["for_each"]
        .as_object()
        .expect("map")
        .keys()
        .collect();
    assert_eq!(keys, ["a", "b"]);
    assert_eq!(m["suffixes"]["length"], 6);
    assert_eq!(
        m["items"]["suffix"],
        "${module.suffixes[each.value.name].result}"
    );
    assert!(
        Path::new(m["items"]["source"].as_str().expect("source"))
            .ends_with("infra/proof/modules/item")
    );
}

#[test]
fn bad_agnostic_references_and_each_blocks_fail() {
    let repo = repo();
    let root = repo.path();
    let cases: [(Value, &str); 6] = [
        (
            json!({ "suffixes": { "module": "agnostic/missing", "over": "items", "for_each": "$item.name" } }),
            "names missing module `agnostic/missing`",
        ),
        (
            json!({ "suffixes": { "module": "agnostic/Bad", "over": "items", "for_each": "$item.name" } }),
            "not a lowercase-dash name",
        ),
        (
            json!({ "suffixes": { "module": "agnostic/suffix", "over": "label", "for_each": "$item.name" } }),
            "`over` must name an array property",
        ),
        (
            json!({ "suffixes": { "module": "agnostic/suffix", "over": "items" } }),
            "needs `for_each`",
        ),
        (
            json!({ "suffixes": { "module": "agnostic/suffix", "over": "items", "for_each": "$item.name", "rotate": { "period_days": 1, "overlap_days": 0 } } }),
            "cannot rotate",
        ),
        (
            json!({ "items": { "module": "agnostic/suffix", "over": "items", "for_each": "$item.name" } }),
            "reuses a property's or singleton's name",
        ),
    ];
    for (each, want) in cases {
        set_each(root, &each);
        assert_fails(root, want);
    }
    set_each(
        root,
        &json!({ "suffixes": { "module": "agnostic/suffix", "over": "items", "for_each": "$item.name", "inputs": { "nope": 1 } } }),
    );
    assert_fails(root, "block `suffixes` sets undeclared input `nope`");
}

#[test]
fn providers_follow_agnostic_modules() {
    let repo = repo();
    let root = repo.path();
    module(
        root,
        "infra/agnostic/modules/suffix",
        &["length"],
        "\nterraform {\n  required_providers {\n    random = {\n      source  = \"hashicorp/random\"\n      version = \"3.9.1\"\n    }\n  }\n}\n",
    );
    write(
        root,
        ".config/providers.yaml",
        "description: Test allowlist.\nproviders:\n  hashicorp/random: 3.9.1\n",
    );
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    let rendered = root.join("main.tf.json");
    fs::write(&rendered, render(root).to_string()).expect("write render");
    assert_eq!(
        cobot::providers::used(&allow, &root.join(UNIT), &rendered),
        ["hashicorp/random"]
    );
    assert_eq!(
        cobot::providers::used(&allow, &root.join(UNIT), &root.join("absent.json")),
        Vec::<&str>::new()
    );
    let units =
        cobot::select(root, &root.join(".config"), &["infra/agnostic".into()]).expect("valid");
    let own = root.join("agnostic.tf.json");
    let json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &allow,
        0,
        &cobot::config::Existing::new(),
    );
    fs::write(&own, json.to_string()).expect("write render");
    assert_eq!(
        cobot::providers::used(&allow, &root.join("infra/agnostic"), &own),
        Vec::<&str>::new(),
        "a module the unit does not compose brings no provider"
    );
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_unit_composing_an_agnostic_module_applies_end_to_end() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = repo();
    let root = repo.path();
    let state = TempDir::new().expect("state");
    let ctx = cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.path().to_path_buf(),
        state_passphrase: STATE_PASS.into(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: 0,
        credentials: None,
        cloudflare_token: None,
        cloudflare_api: None,
    };
    let units = cobot::select(root, &root.join(".config"), &[UNIT.into()]).expect("valid");
    let applied = cobot::apply(&ctx, &units).expect("apply");
    assert_eq!(
        applied[0].counts(),
        (4, 0, 0),
        "{}",
        cobot::summary(&applied)
    );
    let again = cobot::plan(&ctx, &units).expect("replan");
    assert!(!again[0].has_changes(), "{}", cobot::summary(&again));
}

#[test]
fn a_unit_without_blocks_renders_no_module_and_plans_clean() {
    let repo = repo();
    let root = repo.path();
    let units =
        cobot::select(root, &root.join(".config"), &["infra/agnostic".into()]).expect("valid");
    let json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &cobot::providers::Allowlist::default(),
        0,
        &cobot::config::Existing::new(),
    );
    assert!(json.get("module").is_none(), "{json}");
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let state = TempDir::new().expect("state");
    let ctx = cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.path().to_path_buf(),
        state_passphrase: STATE_PASS.into(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: 0,
        credentials: None,
        cloudflare_token: None,
        cloudflare_api: None,
    };
    let plans = cobot::plan(&ctx, &units).expect("plan");
    assert!(!plans[0].has_changes(), "{}", cobot::summary(&plans));
}

/// Points `items` at `module`; see the separation ADR's catalog-composition amendment.
fn set_item_module(root: &Path, module: &str) {
    let mut s = schema(&suffixes());
    s["properties"]["items"]["x-cobot"]["module"] = json!(module);
    write(
        root,
        &format!("{UNIT}/schema.json"),
        &serde_json::to_string_pretty(&s).expect("json"),
    );
}

#[test]
fn any_catalogs_modules_are_reachable() {
    let repo = repo();
    let root = repo.path();
    module(root, "catalog/shop/modules/item", &["name", "suffix"], "");
    set_item_module(root, "shop/item");
    assert_eq!(problems(root), Vec::<String>::new());
    let json = render(root);
    let source = json["module"]["items"]["source"].as_str().expect("source");
    assert!(
        Path::new(source).ends_with("catalog/shop/modules/item"),
        "{source}"
    );
}

#[test]
fn unknown_catalogs_and_modules_fail() {
    let repo = repo();
    let root = repo.path();
    module(root, "catalog/shop/modules/item", &["name", "suffix"], "");
    for (module, want) in [
        ("nope/item", "looked for catalog/nope/modules/item"),
        ("shop/gone", "looked for catalog/shop/modules/gone"),
        ("Shop/item", "not a lowercase-dash name"),
        ("shop/a/b", "not a lowercase-dash name"),
    ] {
        set_item_module(root, module);
        assert_fails(root, want);
    }
}
