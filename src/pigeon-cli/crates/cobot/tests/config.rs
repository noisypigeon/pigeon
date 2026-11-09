//! Config units: validation, `x-cobot` rendering, secrets, and rotation end to end.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

const STATE_PASS: &str = "state-passphrase-0123456789";
const SECRETS_PASS: &str = "secrets-passphrase-0123456789";
const WORKLOAD_PASS: &str = "workload-passphrase-0123456789";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn module(root: &Path, unit: &str, name: &str, vars: &[&str]) {
    let mut tf = String::new();
    for v in vars {
        let _ = write!(tf, "variable \"{v}\" {{\n  type = any\n}}\n");
    }
    let input = if vars.is_empty() {
        "null".to_string()
    } else {
        format!(
            "{{ {} }}",
            vars.iter()
                .map(|v| format!("{v} = var.{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let _ = write!(
        tf,
        "resource \"terraform_data\" \"this\" {{\n  input = {input}\n}}\n\noutput \"id\" {{\n  value = terraform_data.this.id\n}}\n"
    );
    write(root, &format!("{unit}/modules/{name}/main.tf"), &tf);
    write(
        root,
        &format!("{unit}/modules/{name}/module.yaml"),
        "version: 0.1.0\ndescription: A test module.\n",
    );
}

fn schema(secrets: &[&str]) -> Value {
    let mut s = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["label", "items", "tokens", "rotation"],
        "x-cobot": {
            "version": "0.1.0",
            "secrets": secrets,
            "singletons": {
                "single": { "module": "single", "inputs": { "label": "$config.label" } }
            }
        },
        "properties": {
            "label": { "type": "string" },
            "rotation": {
                "type": "object",
                "properties": { "period_days": { "type": "integer" }, "overlap_days": { "type": "integer" } }
            },
            "items": {
                "type": "array",
                "items": { "type": "string" },
                "x-cobot": {
                    "module": "item",
                    "for_each": "$item",
                    "inputs": { "name": "$item", "single": "$module.single.id" }
                }
            },
            "tokens": {
                "type": "array",
                "items": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] },
                "x-cobot": {
                    "module": "token",
                    "for_each": "$item.name",
                    "rotate": { "period_days": "$config.rotation.period_days", "overlap_days": "$config.rotation.overlap_days" },
                    "inputs": {
                        "name": "$item.name",
                        "items": "$module.items[*].id",
                        "not_before": "$generation.not_before",
                        "expires_on": "$generation.expires_on"
                    }
                }
            }
        }
    });
    if !secrets.is_empty() {
        s["properties"]["items"]["x-cobot"]["inputs"]["secret"] =
            json!(format!("$secret.{}", secrets[0]));
    }
    s
}

const CONFIG: &str = "label: proof\nitems: [a, b]\ntokens:\n  - name: deploy\nrotation: { period_days: 90, overlap_days: 7 }\n";

fn repo(unit: &str, secrets: &[&str]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    write(root, &format!("{unit}/config.yaml"), CONFIG);
    write(
        root,
        &format!("{unit}/schema.json"),
        &serde_json::to_string_pretty(&schema(secrets)).expect("json"),
    );
    module(root, unit, "single", &["label"]);
    let mut item_vars = vec!["name", "single"];
    if !secrets.is_empty() {
        item_vars.push("secret");
    }
    module(root, unit, "item", &item_vars);
    module(
        root,
        unit,
        "token",
        &["name", "items", "not_before", "expires_on"],
    );
    dir
}

fn problems(root: &Path) -> Vec<String> {
    cobot::unit::validate(root, &cobot::providers::Allowlist::default())
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

fn ctx(root: &Path, state: &Path, today: i64) -> cobot::Context {
    cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: STATE_PASS.into(),
        secrets_passphrase: Some(SECRETS_PASS.into()),
        workload_secrets_passphrase: Some(WORKLOAD_PASS.into()),
        today,
        credentials: None,
        cloudflare_token: None,
        cloudflare_api: None,
        platform: None,
        adopt: false,
        provider_config: None,
        provider_token: false,
    }
}

#[test]
fn a_valid_config_unit_passes_and_renders() {
    let repo = repo("infra/proof", &[]);
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    let today = cobot::generations::day("2026-11-02").expect("day");
    let json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &cobot::providers::Allowlist::default(),
        today,
        &cobot::config::Existing::new(),
    );
    let m = &json["module"];
    assert_eq!(m["single"]["label"], "proof");
    assert_eq!(m["items"]["for_each"]["a"], "a");
    assert_eq!(m["items"]["name"], "${each.value}");
    assert_eq!(m["items"]["single"], "${module.single.id}");
    let tokens = m["tokens"]["for_each"].as_object().expect("map");
    let start = (today / 90) * 90;
    let want = format!(
        "deploy@{}..{}",
        cobot::generations::date(start),
        cobot::generations::date(start + 97)
    );
    assert_eq!(tokens.keys().collect::<Vec<_>>(), [&want]);
    assert_eq!(
        m["tokens"]["items"],
        "${{ for k, m in module.items : k => m.id }}"
    );
    assert_eq!(
        m["tokens"]["not_before"],
        "${each.value._generation.not_before}"
    );
    assert!(json.get("variable").is_none());
}

#[test]
fn literal_template_sequences_are_escaped() {
    let repo = repo("infra/proof", &[]);
    write(
        repo.path(),
        "infra/proof/config.yaml",
        &CONFIG.replace("label: proof", "label: \"${var.secrets}\""),
    );
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    let json = cobot::render_json(
        &units[0],
        Path::new("/s"),
        &cobot::providers::Allowlist::default(),
        0,
        &cobot::config::Existing::new(),
    );
    assert_eq!(json["module"]["single"]["label"], "$${var.secrets}");
}

#[test]
fn bad_schemas_and_configs_fail_for_their_reasons() {
    type Edit = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "unknown expression",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["inputs"]["name"] = json!("$env.HOME");
            }),
            "unknown expression",
        ),
        (
            "undeclared input",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["inputs"]["nope"] = json!("$item");
            }),
            "undeclared input",
        ),
        (
            "missing output",
            Box::new(|s| {
                s["properties"]["tokens"]["x-cobot"]["inputs"]["items"] =
                    json!("$module.items[*].nope");
            }),
            "no output",
        ),
        (
            "unknown block",
            Box::new(|s| {
                s["properties"]["tokens"]["x-cobot"]["inputs"]["items"] = json!("$module.ghost.id");
            }),
            "unknown block",
        ),
        (
            "missing module",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["module"] = json!("ghost");
            }),
            "missing module",
        ),
        (
            "undeclared secret",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["inputs"]["name"] = json!("$secret.nope");
            }),
            "not in x-cobot.secrets",
        ),
        (
            "generation outside rotation",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["inputs"]["name"] =
                    json!("$generation.not_before");
            }),
            "rotating block",
        ),
        (
            "absent config path",
            Box::new(|s| {
                s["x-cobot"]["singletons"]["single"]["inputs"]["label"] = json!("$config.nope");
            }),
            "absent from config.yaml",
        ),
        (
            "unindexed list reference",
            Box::new(|s| {
                s["properties"]["tokens"]["x-cobot"]["inputs"]["items"] = json!("$module.items.id");
            }),
            "a list block needs",
        ),
        (
            "escaping module",
            Box::new(|s| {
                s["properties"]["items"]["x-cobot"]["module"] = json!("../../escape");
            }),
            "lowercase-dash name",
        ),
        (
            "no version",
            Box::new(|s| {
                s["x-cobot"]["version"] = json!("one");
            }),
            "x-cobot.version",
        ),
    ];
    for (what, edit, want) in cases {
        let repo = repo("infra/proof", &[]);
        let mut s = schema(&[]);
        edit(&mut s);
        write(repo.path(), "infra/proof/schema.json", &s.to_string());
        let got = problems(repo.path());
        assert!(got.iter().any(|p| p.contains(want)), "{what}: {got:?}");
    }
}

#[test]
fn configs_are_schema_checked_and_credential_free() {
    let repo = repo("infra/proof", &[]);
    write(
        repo.path(),
        "infra/proof/config.yaml",
        &CONFIG.replace("label: proof", "label: 7"),
    );
    assert!(
        problems(repo.path())
            .iter()
            .any(|p| p.contains("config.yaml")),
        "schema validation"
    );
    let repo2 = self::repo("infra/proof", &[]);
    write(
        repo2.path(),
        "infra/proof/config.yaml",
        &CONFIG.replace(
            "label: proof",
            "label: abcdefghijklmnopqrstuvwxyz0123456789ABCD",
        ),
    );
    assert!(
        problems(repo2.path())
            .iter()
            .any(|p| p.contains("looks like a credential"))
    );
}

#[test]
fn bad_keys_and_rotations_fail() {
    for (what, config, want) in [
        (
            "duplicate key",
            CONFIG.replace("items: [a, b]", "items: [a, a]"),
            "repeats the key",
        ),
        (
            "empty key",
            CONFIG.replace("items: [a, b]", "items: [a, \"\"]"),
            "non-empty string key",
        ),
        (
            "zero period",
            CONFIG.replace("period_days: 90", "period_days: 0"),
            "positive integer period_days",
        ),
        (
            "negative overlap",
            CONFIG.replace("overlap_days: 7", "overlap_days: -1"),
            "non-negative integer overlap_days",
        ),
    ] {
        let repo = repo("infra/proof", &[]);
        write(repo.path(), "infra/proof/config.yaml", &config);
        let got = problems(repo.path());
        assert!(got.iter().any(|p| p.contains(want)), "{what}: {got:?}");
    }
}

#[test]
fn modules_hold_one_resource() {
    let repo = repo("infra/proof", &[]);
    write(
        repo.path(),
        "infra/proof/modules/item/extra.tf",
        "resource \"terraform_data\" \"two\" {}\n",
    );
    assert!(
        problems(repo.path())
            .iter()
            .any(|p| p.contains("one resource"))
    );
}

#[test]
fn secrets_are_infra_only_and_always_encrypted() {
    let repo = repo("infra/proof", &["token"]);
    assert!(
        problems(repo.path())
            .iter()
            .any(|p| p.contains("secrets.config.yaml")),
        "missing file"
    );
    write(
        repo.path(),
        "infra/proof/secrets.config.yaml",
        "token: plaintext\n",
    );
    assert!(problems(repo.path()).iter().any(|p| p.contains("not ENC")));
    let path = repo.path().join("infra/proof/secrets.config.yaml");
    fs::remove_file(&path).expect("rm");
    cobot::secrets::set(&path, "token", "value-1", SECRETS_PASS).expect("set");
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    cobot::secrets::set(&path, "extra", "value-2", SECRETS_PASS).expect("set");
    assert!(problems(repo.path()).iter().any(|p| p.contains("differ")));

    let workload = self::repo("infra/proof", &["provisioner_token"]);
    let path = workload.path().join("infra/proof/secrets.config.yaml");
    cobot::secrets::set(&path, "provisioner_token", "v", SECRETS_PASS).expect("set");
    assert_eq!(problems(workload.path()), Vec::<String>::new());
    fs::rename(
        workload.path().join("infra"),
        workload.path().join("workloads"),
    )
    .expect("mv");
    let got = problems(workload.path());
    assert!(got.iter().any(|p| p.contains("break-glass")), "{got:?}");
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn rotation_and_secrets_end_to_end() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = repo("infra/proof", &["token"]);
    let secrets = repo.path().join("infra/proof/secrets.config.yaml");
    cobot::secrets::set(&secrets, "token", "very-secret-value-42", SECRETS_PASS).expect("set");
    let state = TempDir::new().expect("tempdir");
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    let day = |s| cobot::generations::day(s).expect("day");
    let start = (day("2026-11-02") / 90) * 90;

    let plans = cobot::plan(&ctx(repo.path(), state.path(), start + 5), &units).expect("plan");
    let summary = cobot::summary(&plans);
    assert_eq!(plans[0].counts(), (4, 0, 0), "{summary}");
    assert!(!summary.contains("very-secret-value-42"), "{summary}");

    cobot::apply(&ctx(repo.path(), state.path(), start + 5), &units).expect("apply");
    let written =
        fs::read_to_string(state.path().join("infra/proof/terraform.tfstate")).expect("state");
    assert!(!written.contains("very-secret-value-42") && written.contains("encrypted_data"));

    let same = cobot::plan(&ctx(repo.path(), state.path(), start + 6), &units).expect("replan");
    assert!(!same[0].has_changes(), "{}", cobot::summary(&same));

    let next = start + 90;
    let rotated = cobot::apply(&ctx(repo.path(), state.path(), next + 2), &units).expect("rotate");
    assert_eq!(
        rotated[0].counts(),
        (1, 0, 0),
        "{}",
        cobot::summary(&rotated)
    );

    let expired = cobot::apply(&ctx(repo.path(), state.path(), next + 8), &units).expect("expire");
    assert_eq!(
        expired[0].counts(),
        (0, 0, 1),
        "{}",
        cobot::summary(&expired)
    );

    let mut no_pass = ctx(repo.path(), state.path(), next + 8);
    no_pass.secrets_passphrase = None;
    assert!(matches!(
        cobot::plan(&no_pass, &units),
        Err(cobot::Error::SecretsPassphrase)
    ));
}
