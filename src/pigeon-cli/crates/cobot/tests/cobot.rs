//! Units, rendering, and an end-to-end plan and apply when `tofu` is on `PATH`.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

const PASSPHRASE: &str = "test-passphrase-0123456789";

const PROOF: &str = "variable \"message\" {\n  type = string\n}\n\nresource \"terraform_data\" \"this\" {\n  input = var.message\n}\n";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn unit_yaml(name: &str, kind: &str, source: &str) -> String {
    format!(
        "schema: 0\nname: {name}\nkind: {kind}\nmodules:\n  - name: proof\n    source: {source}\n    inputs: {{ message: hello }}\n"
    )
}

fn proof_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    write(
        dir.path(),
        "infra/bootstrap/unit.yaml",
        &unit_yaml("bootstrap", "infra", "./modules/proof"),
    );
    write(dir.path(), "infra/bootstrap/modules/proof/main.tf", PROOF);
    dir
}

fn messages(root: &Path) -> Vec<String> {
    cobot::unit::validate(root, &cobot::providers::Allowlist::default())
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

#[test]
fn a_valid_unit_passes() {
    let repo = proof_repo();
    assert_eq!(messages(repo.path()), Vec::<String>::new());
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].path, "infra/bootstrap");
}

#[test]
fn invalid_units_fail_for_the_right_reason() {
    let cases: [(&str, &str, &str); 9] = [
        (
            "unknown key",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules: []\nextra: 1\n",
        ),
        (
            "schema",
            "infra/u/unit.yaml",
            "schema: 1\nname: u\nkind: infra\nmodules:\n  - { name: proof, source: ./m }\n",
        ),
        (
            "name",
            "infra/u/unit.yaml",
            "schema: 0\nname: Bad_Name\nkind: infra\nmodules:\n  - { name: proof, source: ./m }\n",
        ),
        (
            "kind",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: workload\nmodules:\n  - { name: proof, source: ./m }\n",
        ),
        (
            "no modules",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules: []\n",
        ),
        (
            "repeat",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules:\n  - { name: proof, source: ./m }\n  - { name: proof, source: ./m }\n",
        ),
        (
            "leaves",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules:\n  - { name: proof, source: ../x }\n",
        ),
        (
            "missing source",
            "infra/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules:\n  - { name: proof, source: ./nope }\n",
        ),
        (
            "workload kind",
            "workloads/u/unit.yaml",
            "schema: 0\nname: u\nkind: infra\nmodules:\n  - { name: proof, source: ./m }\n",
        ),
    ];
    let expect = [
        "unknown field",
        "schema 1 is unknown",
        "not lowercase-dash",
        "kind does not match",
        "declares no modules",
        "repeats",
        "leaves the unit",
        "not a directory",
        "kind does not match",
    ];
    for ((what, path, yaml), want) in cases.iter().zip(expect) {
        let dir = TempDir::new().expect("tempdir");
        write(dir.path(), path, yaml);
        let unit_dir = Path::new(path)
            .parent()
            .expect("parent")
            .to_str()
            .expect("utf-8");
        write(dir.path(), &format!("{unit_dir}/m/main.tf"), PROOF);
        let got = messages(dir.path());
        assert!(got.iter().any(|m| m.contains(want)), "{what}: {got:?}");
    }
}

#[test]
fn hcl_that_reads_the_runner_is_rejected() {
    for (bad, want) in [
        (
            "resource \"terraform_data\" \"x\" {\n  provisioner \"local-exec\" { command = \"id\" }\n}\n",
            "provisioners",
        ),
        (
            "output \"o\" { value = file(\"/proc/self/environ\") }\n",
            "`file()`",
        ),
        (
            "output \"o\" { value = templatefile (\"a\", {}) }\n",
            "`templatefile()`",
        ),
        (
            "output \"o\" { value = pathexpand(\"~\") }\n",
            "`pathexpand()`",
        ),
    ] {
        let repo = proof_repo();
        write(repo.path(), "infra/bootstrap/modules/proof/extra.tf", bad);
        let got = messages(repo.path());
        assert!(got.iter().any(|m| m.contains(want)), "{want}: {got:?}");
    }
}

#[test]
fn units_do_not_nest() {
    let repo = proof_repo();
    write(
        repo.path(),
        "infra/bootstrap/inner/unit.yaml",
        &unit_yaml("inner", "infra", "./m"),
    );
    write(repo.path(), "infra/bootstrap/inner/m/main.tf", PROOF);
    assert!(
        messages(repo.path())
            .iter()
            .any(|m| m.contains("do not nest"))
    );
}

#[test]
fn render_points_modules_and_state_at_absolute_paths() {
    let repo = proof_repo();
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    let json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &cobot::providers::Allowlist::default(),
        0,
        &cobot::config::Existing::new(),
    );
    assert_eq!(
        json["terraform"]["backend"]["local"]["path"],
        "/state/infra/bootstrap/terraform.tfstate"
    );
    assert_eq!(json["module"]["proof"]["message"], "hello");
    assert!(
        json["module"]["proof"]["source"]
            .as_str()
            .expect("source")
            .ends_with("infra/bootstrap/./modules/proof")
    );
}

#[test]
fn passphrase_is_required() {
    assert!(cobot::passphrase(None).is_err());
    assert!(cobot::passphrase(Some("short".into())).is_err());
    assert!(cobot::passphrase(Some(PASSPHRASE.into())).is_ok());
}

fn ctx(root: &Path, state: &Path, passphrase: &str) -> cobot::Context {
    cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: passphrase.to_string(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: 0,
        credentials: None,
        cloudflare_token: None,
        cloudflare_api: None,
        platform: None,
        adopt: false,
        provider_config: None,
    }
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn plan_and_apply_write_only_encrypted_state() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = proof_repo();
    let state = TempDir::new().expect("tempdir");
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");

    let plans = cobot::plan(&ctx(repo.path(), state.path(), PASSPHRASE), &units).expect("plan");
    assert_eq!(plans[0].changes.len(), 1);
    assert_eq!(
        plans[0].changes[0].address,
        "module.proof.terraform_data.this"
    );
    let summary = cobot::summary(&plans);
    assert!(
        summary.contains("create `module.proof.terraform_data.this`"),
        "{summary}"
    );
    assert!(
        !summary.contains("hello"),
        "summary leaked a value: {summary}"
    );

    cobot::apply(&ctx(repo.path(), state.path(), PASSPHRASE), &units).expect("apply");
    let written = fs::read_to_string(state.path().join("infra/bootstrap/terraform.tfstate"))
        .expect("state written");
    assert!(written.contains("encrypted_data"), "state is not encrypted");
    assert!(!written.contains("hello"), "state holds plaintext");
    assert!(!written.contains("terraform_data"), "state holds plaintext");

    let again = cobot::plan(&ctx(repo.path(), state.path(), PASSPHRASE), &units).expect("replan");
    assert!(!again[0].has_changes());

    let wrong = cobot::plan(
        &ctx(repo.path(), state.path(), "another-passphrase-0123"),
        &units,
    );
    let err = wrong.expect_err("wrong passphrase fails").to_string();
    assert!(
        !err.contains(PASSPHRASE) && !err.contains("another-passphrase-0123"),
        "{err}"
    );
}

#[test]
fn other_config_formats_and_symlinks_are_rejected() {
    for (path, want) in [
        (
            "infra/bootstrap/modules/proof/x.tf.json",
            "only `.tf` files",
        ),
        ("infra/bootstrap/modules/proof/x.tofu", "only `.tf` files"),
        (
            "infra/bootstrap/modules/proof/x.tofu.json",
            "only `.tf` files",
        ),
    ] {
        let repo = proof_repo();
        write(repo.path(), path, "{}");
        let got = messages(repo.path());
        assert!(got.iter().any(|m| m.contains(want)), "{path}: {got:?}");
    }
    #[cfg(unix)]
    {
        let repo = proof_repo();
        let outside = TempDir::new().expect("tempdir");
        write(outside.path(), "evil/main.tf", PROOF);
        std::os::unix::fs::symlink(
            outside.path().join("evil"),
            repo.path().join("infra/bootstrap/modules/linked"),
        )
        .expect("symlink");
        let got = messages(repo.path());
        assert!(
            got.iter().any(|m| m.contains("symlinks are not allowed")),
            "{got:?}"
        );
    }
}

#[test]
fn a_unit_without_providers_plans_beside_a_lockfile() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = proof_repo();
    write(
        repo.path(),
        ".config/providers.yaml",
        "description: x\nproviders:\n  cloudflare/cloudflare: 5.28.0\n",
    );
    write(
        repo.path(),
        ".config/providers.lock.hcl",
        "provider \"registry.opentofu.org/cloudflare/cloudflare\" {\n  version     = \"5.28.0\"\n  constraints = \"5.28.0\"\n  hashes = [\n    \"h1:3MScc9whgqTm8wB64P4pb4M3zzm8JHylgpnQGSvPjBE=\",\n  ]\n}\n",
    );
    let unit = repo.path().join("infra/bootstrap/unit.yaml");
    let yaml = fs::read_to_string(&unit).expect("read");
    assert!(yaml.contains("hello"), "{yaml}");
    fs::write(&unit, yaml.replace("hello", "cloudflare/cloudflare")).expect("write");
    let state = TempDir::new().expect("tempdir");
    let units = cobot::select(repo.path(), &repo.path().join(".config"), &[]).expect("valid");
    let plans = cobot::plan(&ctx(repo.path(), state.path(), PASSPHRASE), &units).expect("plan");
    assert_eq!(plans[0].changes.len(), 1);
}
