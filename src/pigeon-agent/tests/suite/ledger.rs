//! The grants ledger on this repository; see
//! `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.

use std::path::PathBuf;

use agent::infra::catalog::{ledger as read, ledger_resolved as resolve};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_repositorys_ledger_reads_and_checks_clean_against_its_catalogs() {
    let root = repo_root();
    assert!(read(&root, &root).expect("checks clean").is_some());
}

#[test]
fn an_instance_without_a_ledger_has_none_and_a_bad_one_is_named() {
    let tmp = tempfile::tempdir().expect("tmp");
    assert_eq!(read(tmp.path(), &repo_root()), Ok(None));
    std::fs::create_dir_all(tmp.path().join(".config")).expect("dir");
    std::fs::write(
        tmp.path().join(".config/grants.yaml"),
        "grants:\n  x:\n    mint: { capabilities: [nope], ttl: 1h }\n    route: { job: infra.apply }\n",
    )
    .expect("writes");
    let problems = read(tmp.path(), &repo_root()).expect_err("unknown capability");
    assert!(
        problems.iter().any(|p| p.contains("`nope`")),
        "{problems:?}"
    );
}

#[test]
fn the_repositorys_ledger_resolves_at_a_commit() {
    let root = repo_root();
    let r = resolve(&root, &root, "abc")
        .expect("resolves")
        .expect("a ledger");
    assert_eq!(r.sha, "abc");
    for g in r.ledger.grants.keys() {
        let _ = r.wanted(g);
    }
}

// See `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
#[test]
fn the_deployed_ledger_carries_each_platform_units_capabilities() {
    let root = repo_root();
    let r = resolve(&root, &root, "abc")
        .expect("resolves")
        .expect("a ledger");
    let wanted = r.unit("platform/blue").expect("platform/blue's set");
    let names: Vec<&str> = wanted.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "access-apps.write",
            "access-tokens.write",
            "account.read",
            "aigateway.write",
            "d1.write",
            "queues.write",
            "r2.write",
            "secrets-store.read",
            "workers.read",
        ]
    );
    assert!(
        wanted
            .iter()
            .all(|w| !w.permissions.is_empty() && w.scope == "account"),
        "{wanted:?}"
    );
    assert!(
        r.wanted("platform-run")
            .is_err_and(|e| e.contains("unit run"))
    );
}
