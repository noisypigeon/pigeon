//! `pigeon cobot` end to end.
#![cfg(feature = "cobot")]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cobot() -> Command {
    let mut cmd = Command::cargo_bin("pigeon").expect("binary");
    cmd.arg("cobot").env_remove("COBOT_STATE_PASSPHRASE");
    cmd
}

#[test]
fn validate_passes_on_an_empty_repository() {
    let root = TempDir::new().expect("tempdir");
    cobot()
        .args(["validate", "--root"])
        .arg(root.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("0 unit(s) valid"));
}

#[test]
fn validate_fails_with_exit_1() {
    let root = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(root.path().join("infra/u")).expect("mkdir");
    std::fs::write(
        root.path().join("infra/u/unit.yaml"),
        "schema: 0\nname: u\nkind: workload\nmodules: []\n",
    )
    .expect("write");
    cobot()
        .args(["validate", "--root"])
        .arg(root.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("kind does not match"));
}

#[test]
fn capabilities_print_json_and_reject_invalid_files() {
    let root = TempDir::new().expect("tempdir");
    let caps = root.path().join("catalog/x/capabilities.yaml");
    std::fs::create_dir_all(caps.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &caps,
        "description: X.\ncapabilities:\n  dns.write: { permissions: [DNS Write], scope: zone }\n",
    )
    .expect("write");
    let out = cobot()
        .args(["capabilities", "--root"])
        .arg(root.path())
        .assert()
        .code(0);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    assert_eq!(json[0]["provider"], "x");
    assert_eq!(json[0]["capabilities"]["dns.write"]["scope"], "zone");
    std::fs::write(&caps, "description: X.\ncapabilities: {}\n").expect("write");
    cobot()
        .args(["capabilities", "--root"])
        .arg(root.path())
        .assert()
        .code(1)
        .stderr(predicate::str::contains("declares no capabilities"));
}

#[test]
fn plan_refuses_without_a_passphrase() {
    let root = TempDir::new().expect("tempdir");
    cobot()
        .args(["plan", "--root"])
        .arg(root.path())
        .args(["--state"])
        .arg(root.path().join("state"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "COBOT_STATE_PASSPHRASE must be set",
        ));
}

#[test]
fn cobot_reports_its_own_version() {
    cobot()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(cobot::VERSION));
}

#[test]
fn render_with_relative_root_and_state_writes_absolute_paths() {
    let repo = TempDir::new().expect("tempdir");
    let unit = repo.path().join("infra/bootstrap");
    std::fs::create_dir_all(unit.join("modules/proof")).expect("mkdir");
    std::fs::write(
        unit.join("unit.yaml"),
        "schema: 0\nname: bootstrap\nkind: infra\nmodules:\n  - { name: proof, source: ./modules/proof }\n",
    )
    .expect("write");
    std::fs::write(
        unit.join("modules/proof/main.tf"),
        "resource \"terraform_data\" \"this\" {}\n",
    )
    .expect("write");
    cobot()
        .current_dir(repo.path())
        .args(["render", "--state", "./state"])
        .assert()
        .code(0);
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.path().join(".cobot/infra/bootstrap/main.tf.json"))
            .expect("rendered"),
    )
    .expect("json");
    let state = json["terraform"]["backend"]["local"]["path"]
        .as_str()
        .expect("path");
    let source = json["module"]["proof"]["source"].as_str().expect("source");
    assert!(std::path::Path::new(state).is_absolute(), "{state}");
    assert!(std::path::Path::new(source).is_dir(), "{source}");
}

#[test]
fn plan_names_each_tofu_step_on_stderr() {
    if !std::process::Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = TempDir::new().expect("tempdir");
    let unit = repo.path().join("infra/bootstrap");
    std::fs::create_dir_all(unit.join("modules/proof")).expect("mkdir");
    std::fs::write(
        unit.join("unit.yaml"),
        "schema: 0\nname: bootstrap\nkind: infra\nmodules:\n  - { name: proof, source: ./modules/proof }\n",
    )
    .expect("write");
    std::fs::write(
        unit.join("modules/proof/main.tf"),
        "resource \"terraform_data\" \"this\" {}\n",
    )
    .expect("write");
    let passphrase = "progress-passphrase-0123456789";
    let output = cobot()
        .current_dir(repo.path())
        .env("COBOT_STATE_PASSPHRASE", passphrase)
        .args(["plan", "--state", "./state"])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    for line in [
        "cobot: infra/bootstrap: tofu init…",
        "cobot: infra/bootstrap: tofu plan…",
    ] {
        assert!(stderr.contains(line), "missing {line:?} in {stderr}");
    }
    assert!(stderr.contains("took"), "{stderr}");
    assert!(!stderr.contains(passphrase), "{stderr}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("cobot: infra/bootstrap: tofu"),
        "progress stays off stdout"
    );
}
