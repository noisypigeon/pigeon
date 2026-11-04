//! Stored plans, end to end when `tofu` is on `PATH`; see the phase 3 amendment §5 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

const PASSPHRASE: &str = "test-passphrase-0123456789";
const NOW: u64 = 1_800_000_000;

const PROOF: &str = "variable \"message\" {\n  type = string\n}\n\nresource \"terraform_data\" \"this\" {\n  input = var.message\n}\n";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn set_message(root: &Path, message: &str) {
    write(
        root,
        "infra/bootstrap/unit.yaml",
        &format!(
            "schema: 0\nname: bootstrap\nkind: infra\nmodules:\n  - name: proof\n    source: ./modules/proof\n    inputs: {{ message: {message} }}\n"
        ),
    );
}

fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    set_message(dir.path(), "hello");
    write(dir.path(), "infra/bootstrap/modules/proof/main.tf", PROOF);
    dir
}

fn ctx(root: &Path, state: &Path) -> cobot::Context {
    cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: PASSPHRASE.into(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: 0,
        credentials: None,
    }
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn units(root: &Path) -> Vec<cobot::Unit> {
    cobot::select(root, &root.join(".config"), &[]).expect("valid")
}

#[test]
fn a_stored_plan_applies_exactly_once() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = repo();
    let state = TempDir::new().expect("tempdir");
    let c = ctx(repo.path(), state.path());
    let (stored, planned) = cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    assert_eq!(stored.units.len(), 1);
    assert_eq!(stored.expires, NOW + cobot::plans::LIFETIME_SECS);
    assert_eq!(stored.units[0].changes, planned[0].changes);
    let file = state
        .path()
        .join("plans")
        .join(&stored.id)
        .join("infra/bootstrap/plan.tfplan");
    let bytes = fs::read(&file).expect("plan stored");
    assert!(
        !bytes.windows(5).any(|w| w == b"hello"),
        "the stored plan is encrypted"
    );
    assert!(
        !state
            .path()
            .join("infra/bootstrap/terraform.tfstate")
            .exists(),
        "planning applies nothing"
    );

    let applied = cobot::apply_stored(&c, &stored.id, NOW + 60).expect("apply");
    assert_eq!(applied[0].changes, planned[0].changes);
    assert!(
        state
            .path()
            .join("infra/bootstrap/terraform.tfstate")
            .exists()
    );
    assert!(!state.path().join("plans").join(&stored.id).exists());
    let again = cobot::apply_stored(&c, &stored.id, NOW + 60).expect_err("applied once");
    assert!(again.to_string().contains("no stored plan"), "{again}");
    assert!(!cobot::plan(&c, &units(repo.path())).expect("replan")[0].has_changes());
}

#[test]
fn expired_changed_unknown_and_stale_plans_are_refused() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = repo();
    let state = TempDir::new().expect("tempdir");
    let c = ctx(repo.path(), state.path());

    let (old, _) = cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    let expired =
        cobot::apply_stored(&c, &old.id, NOW + cobot::plans::LIFETIME_SECS).expect_err("expired");
    assert!(expired.to_string().contains("expired"), "{expired}");

    set_message(repo.path(), "changed");
    let changed = cobot::apply_stored(&c, &old.id, NOW + 60).expect_err("changed");
    assert!(
        changed.to_string().contains("desired state changed"),
        "{changed}"
    );
    set_message(repo.path(), "hello");

    for bad in [
        "../x",
        "ABCDEF0123456789ABCDEF0123456789",
        "abc",
        "00000000000000000000000000000000",
    ] {
        let err = cobot::apply_stored(&c, bad, NOW)
            .expect_err(bad)
            .to_string();
        assert!(err.starts_with("plan: "), "{bad}: {err}");
    }

    let (first, _) = cobot::plan_stored(&c, &units(repo.path()), NOW + 1).expect("plan a");
    let (second, _) = cobot::plan_stored(&c, &units(repo.path()), NOW + 2).expect("plan b");
    assert_ne!(first.id, second.id);
    cobot::apply_stored(&c, &second.id, NOW + 60).expect("apply b");
    let outdated = cobot::apply_stored(&c, &first.id, NOW + 60).expect_err("stale");
    assert!(
        matches!(outdated, cobot::Error::Tofu { .. }),
        "tofu refuses a plan whose state moved on: {outdated}"
    );
}

#[test]
fn a_module_edit_invalidates_a_stored_plan() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = repo();
    let state = TempDir::new().expect("tempdir");
    let c = ctx(repo.path(), state.path());
    let (stored, _) = cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    write(
        repo.path(),
        "infra/bootstrap/modules/proof/main.tf",
        &format!("{PROOF}\n# edited\n"),
    );
    let err = cobot::apply_stored(&c, &stored.id, NOW + 60).expect_err("module changed");
    assert!(err.to_string().contains("desired state changed"), "{err}");
    assert!(
        !state
            .path()
            .join("infra/bootstrap/terraform.tfstate")
            .exists(),
        "nothing applied"
    );
}

#[test]
fn a_retry_skips_units_already_applied() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    for name in ["a", "b"] {
        write(
            root,
            &format!("infra/{name}/unit.yaml"),
            &format!(
                "schema: 0\nname: {name}\nkind: infra\nmodules:\n  - name: proof\n    source: ./modules/proof\n    inputs: {{ message: hello }}\n"
            ),
        );
        write(root, &format!("infra/{name}/modules/proof/main.tf"), PROOF);
    }
    let state = TempDir::new().expect("tempdir");
    let c = ctx(root, state.path());
    let (stored, _) = cobot::plan_stored(&c, &units(root), NOW).expect("plan");
    assert_eq!(stored.units.len(), 2);

    let b_plan = state
        .path()
        .join("plans")
        .join(&stored.id)
        .join("infra/b/plan.tfplan");
    let saved = fs::read(&b_plan).expect("b's plan");
    fs::remove_file(&b_plan).expect("make b fail");
    cobot::apply_stored(&c, &stored.id, NOW + 60).expect_err("b fails");
    assert!(
        state.path().join("infra/a/terraform.tfstate").exists(),
        "a applied"
    );
    assert!(
        !state.path().join("infra/b/terraform.tfstate").exists(),
        "b did not"
    );

    fs::write(&b_plan, saved).expect("restore b's plan");
    let applied = cobot::apply_stored(&c, &stored.id, NOW + 120).expect("retry");
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].unit, "infra/a");
    assert_eq!(
        applied[0].note.as_deref(),
        Some("Already applied from this plan.")
    );
    assert_eq!(applied[1].unit, "infra/b");
    assert!(state.path().join("infra/b/terraform.tfstate").exists());
    assert!(!state.path().join("plans").join(&stored.id).exists());
}
