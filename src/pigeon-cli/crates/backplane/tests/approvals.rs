//! Approval tiers from `.config/approvals.yaml`; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md`.

use backplane::approvals::{Facts, Rules, Tier, Tiers, classify, matches};

fn rules() -> Rules {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../.config/approvals.yaml");
    serde_saphyr::from_str(&std::fs::read_to_string(path).expect("read")).expect("parses")
}

fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_string()).collect()
}

fn tiers_of(scope: &[&str], files: &[&str], facts: Facts) -> Tiers {
    classify(&rules(), &paths(scope), &paths(files), facts)
}

const PLAIN: Facts = Facts {
    deploys_worker: false,
    appends_only: false,
};

#[test]
fn globs_match_within_and_across_segments() {
    assert!(matches(
        "tooling/spikes/**",
        "tooling/spikes/buildkit/run.sh"
    ));
    assert!(matches(
        "**/secrets.config.yaml",
        "infra/cloudflare/secrets.config.yaml"
    ));
    assert!(matches("**/secrets.config.yaml", "secrets.config.yaml"));
    assert!(matches(
        ".claude/settings*.json",
        ".claude/settings.local.json"
    ));
    assert!(!matches(
        ".claude/settings*.json",
        ".claude/x/settings.json"
    ));
    assert!(matches(
        "**/migrations/**",
        "src/pigeon-cli/crates/backplane/migrations/0001.sql"
    ));
    assert!(!matches(".config/policies/**", ".config/approvals.yaml"));
}

#[test]
fn a_new_adr_is_confirmed() {
    let adr = [".claude/decisions/2026-11-08-security-x.md", "CLAUDE.md"];
    let tiers = tiers_of(&adr, &adr, PLAIN);
    assert_eq!(tiers.authorize.tier, Tier::Confirm);
    assert_eq!(tiers.merge.tier, Tier::Confirm);
}

#[test]
fn a_spike_is_authorized_and_closed_automatically_but_never_merged_so() {
    let tiers = tiers_of(
        &["tooling/spikes/buildkit/**"],
        &["tooling/spikes/buildkit/run.sh"],
        PLAIN,
    );
    assert_eq!(tiers.authorize.tier, Tier::Auto);
    assert_eq!(tiers.merge.tier, Tier::Confirm);
    assert_eq!(tiers.close.tier, Tier::Auto);
}

#[test]
fn what_deploys_needs_the_code() {
    let dockerfile = ["src/pigeon-cli/Dockerfile"];
    let tiers = tiers_of(&dockerfile, &dockerfile, PLAIN);
    assert_eq!(tiers.authorize.tier, Tier::Code);
    assert_eq!(tiers.merge.tier, Tier::Code);
    let worker = tiers_of(
        &["src/pigeon-cli/crates/backplane/**"],
        &["src/pigeon-cli/crates/backplane/src/ship.rs"],
        Facts {
            deploys_worker: true,
            appends_only: false,
        },
    );
    assert_eq!(worker.merge.tier, Tier::Code);
    assert_eq!(worker.merge.rule, "deploys");
    assert_eq!(worker.authorize.tier, Tier::Code);
    assert!(
        worker.authorize.rule.starts_with("guarded"),
        "a scope over backplane's crate reaches the approval files: {}",
        worker.authorize.rule
    );
}

#[test]
fn an_amendment_merges_automatically() {
    let adr = [".claude/decisions/2026-11-07-development-job-images-on-cloudflare.md"];
    let tiers = tiers_of(
        &adr,
        &adr,
        Facts {
            deploys_worker: false,
            appends_only: true,
        },
    );
    assert_eq!(tiers.merge.tier, Tier::Auto);
    assert_eq!(tiers.merge.rule, "ADR amendment");
}

#[test]
fn the_rules_and_every_policy_always_need_the_code() {
    for file in [
        ".config/approvals.yaml",
        ".config/policies/change.yaml",
        ".mcp.json",
    ] {
        let tiers = tiers_of(
            &[file],
            &[file],
            Facts {
                deploys_worker: false,
                appends_only: true,
            },
        );
        assert_eq!(tiers.authorize.tier, Tier::Code, "{file}");
        assert_eq!(tiers.merge.tier, Tier::Code, "{file}");
    }
}
