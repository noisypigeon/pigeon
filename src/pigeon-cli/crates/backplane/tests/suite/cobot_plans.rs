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

fn ctx(root: &Path, state: &Path) -> backplane::cobot::Context {
    backplane::cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: PASSPHRASE.into(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: 0,
        credentials: None,
        cloudflare_token: None,
        cloudflare_api: None,
        platform: None,
        adopt: false,
        provider_config: None,
        provider_token: false,
    }
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn units(root: &Path) -> Vec<backplane::cobot::Unit> {
    backplane::cobot::select(root, &root.join(".config"), &[]).expect("valid")
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
    let (stored, planned) =
        backplane::cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    assert_eq!(stored.units.len(), 1);
    assert_eq!(stored.expires, NOW + backplane::cobot::plans::LIFETIME_SECS);
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

    let applied = backplane::cobot::apply_stored(&c, &stored.id, NOW + 60).expect("apply");
    assert_eq!(applied[0].changes, planned[0].changes);
    let tfstate = state.path().join("infra/bootstrap/terraform.tfstate");
    let after = fs::read(&tfstate).expect("applied");
    assert!(!state.path().join("plans").join(&stored.id).exists());
    let again = backplane::cobot::apply_stored(&c, &stored.id, NOW + 120)
        .expect("a completed plan's result");
    assert_eq!(again[0].changes, planned[0].changes);
    assert!(
        again[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("Applied from this plan earlier.")),
        "{again:?}"
    );
    assert_eq!(
        fs::read(&tfstate).expect("state"),
        after,
        "nothing applied twice"
    );
    let gone = backplane::cobot::apply_stored(
        &c,
        &stored.id,
        NOW + 60 + backplane::cobot::plans::COMPLETED_SECS,
    )
    .expect_err("the record expires");
    assert!(gone.to_string().contains("no stored plan"), "{gone}");
    assert!(!backplane::cobot::plan(&c, &units(repo.path())).expect("replan")[0].has_changes());
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

    let (old, _) = backplane::cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    let expired =
        backplane::cobot::apply_stored(&c, &old.id, NOW + backplane::cobot::plans::LIFETIME_SECS)
            .expect_err("expired");
    assert!(expired.to_string().contains("expired"), "{expired}");

    set_message(repo.path(), "changed");
    let changed = backplane::cobot::apply_stored(&c, &old.id, NOW + 60).expect_err("changed");
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
        let err = backplane::cobot::apply_stored(&c, bad, NOW)
            .expect_err(bad)
            .to_string();
        assert!(err.starts_with("plan: "), "{bad}: {err}");
    }

    let (first, _) =
        backplane::cobot::plan_stored(&c, &units(repo.path()), NOW + 1).expect("plan a");
    let (second, _) =
        backplane::cobot::plan_stored(&c, &units(repo.path()), NOW + 2).expect("plan b");
    assert_ne!(first.id, second.id);
    backplane::cobot::apply_stored(&c, &second.id, NOW + 60).expect("apply b");
    let outdated = backplane::cobot::apply_stored(&c, &first.id, NOW + 60).expect_err("stale");
    assert!(
        matches!(outdated, backplane::cobot::Error::Tofu { .. }),
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
    let (stored, _) = backplane::cobot::plan_stored(&c, &units(repo.path()), NOW).expect("plan");
    write(
        repo.path(),
        "infra/bootstrap/modules/proof/main.tf",
        &format!("{PROOF}\n# edited\n"),
    );
    let err = backplane::cobot::apply_stored(&c, &stored.id, NOW + 60).expect_err("module changed");
    assert!(err.to_string().contains("desired state changed"), "{err}");
    assert!(
        !state
            .path()
            .join("infra/bootstrap/terraform.tfstate")
            .exists(),
        "nothing applied"
    );
}

/// A repository with units `infra/a` and `infra/b`.
fn two_units() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for name in ["a", "b"] {
        write(
            dir.path(),
            &format!("infra/{name}/unit.yaml"),
            &format!(
                "schema: 0\nname: {name}\nkind: infra\nmodules:\n  - name: proof\n    source: ./modules/proof\n    inputs: {{ message: hello }}\n"
            ),
        );
        write(
            dir.path(),
            &format!("infra/{name}/modules/proof/main.tf"),
            PROOF,
        );
    }
    dir
}

#[test]
fn checkpoints_follow_each_unit_and_precede_the_plans_removal() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let dir = two_units();
    let root = dir.path();
    let state = TempDir::new().expect("tempdir");
    let s = state.path();
    let c = ctx(root, s);
    let (stored, _) = backplane::cobot::plan_stored(&c, &units(root), NOW).expect("plan");
    let plan = s.join("plans").join(&stored.id);
    let record = s
        .join("plans/completed")
        .join(format!("{}.json", stored.id));
    let mut seen = Vec::new();
    backplane::cobot::apply_stored_with(&c, &stored.id, NOW + 60, &mut || {
        seen.push((
            plan.join("infra/a/applied").is_file(),
            plan.join("infra/b/applied").is_file(),
            record.is_file(),
            plan.exists(),
        ));
        Ok(())
    })
    .expect("apply");
    assert_eq!(
        seen,
        [
            (true, false, false, true),
            (true, true, false, true),
            (true, true, true, true),
        ],
        "after a, after b, then after the record and before the plan's removal"
    );
    assert!(!plan.exists() && record.is_file());

    let (stored, _) = {
        fs::write(root.join("infra/a/modules/proof/extra.tf"), "# changed\n").expect("change");
        backplane::cobot::plan_stored(&c, &units(root), NOW + 300).expect("plan again")
    };
    let mut calls = 0;
    let stopped = backplane::cobot::apply_stored_with(&c, &stored.id, NOW + 360, &mut || {
        calls += 1;
        if calls == 3 {
            return Err(backplane::cobot::Error::State(
                "stopped before the plan's removal".into(),
            ));
        }
        Ok(())
    });
    assert!(stopped.is_err(), "stopped after the record");
    let plan = s.join("plans").join(&stored.id);
    assert!(plan.exists(), "the plan's files remain");
    let late = NOW + 300 + backplane::cobot::plans::LIFETIME_SECS + 60;
    let reported = backplane::cobot::apply_stored(&c, &stored.id, late)
        .expect("a completed plan, past its expiry");
    assert_eq!(reported.len(), 2);
    assert!(
        reported
            .iter()
            .all(|u| u.note.as_deref().is_some_and(|n| n.contains("earlier"))),
        "{reported:?}"
    );
    assert!(!plan.exists(), "what was left of the plan is removed");

    let (stored, _) = {
        fs::write(root.join("infra/b/modules/proof/extra.tf"), "# changed\n").expect("change");
        backplane::cobot::plan_stored(&c, &units(root), NOW + 600).expect("plan a third time")
    };
    let mut calls = 0;
    let err = backplane::cobot::apply_stored_with(&c, &stored.id, NOW + 660, &mut || {
        calls += 1;
        Err(backplane::cobot::Error::State(
            "R2 refused the write".into(),
        ))
    })
    .expect_err("a failed write back stops the run");
    assert!(err.to_string().contains("R2 refused"), "{err}");
    assert_eq!(calls, 1, "nothing runs after a failed write back");
}

#[test]
fn a_retry_skips_units_already_applied() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let dir = two_units();
    let root = dir.path();
    let state = TempDir::new().expect("tempdir");
    let c = ctx(root, state.path());
    let (stored, _) = backplane::cobot::plan_stored(&c, &units(root), NOW).expect("plan");
    assert_eq!(stored.units.len(), 2);

    let b_plan = state
        .path()
        .join("plans")
        .join(&stored.id)
        .join("infra/b/plan.tfplan");
    let saved = fs::read(&b_plan).expect("b's plan");
    fs::remove_file(&b_plan).expect("make b fail");
    backplane::cobot::apply_stored(&c, &stored.id, NOW + 60).expect_err("b fails");
    assert!(
        state.path().join("infra/a/terraform.tfstate").exists(),
        "a applied"
    );
    assert!(
        !state.path().join("infra/b/terraform.tfstate").exists(),
        "b did not"
    );

    fs::write(&b_plan, saved).expect("restore b's plan");
    let applied = backplane::cobot::apply_stored(&c, &stored.id, NOW + 120).expect("retry");
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
