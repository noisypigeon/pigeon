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
    adds_secret: false,
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
            adds_secret: false,
        },
    );
    assert_eq!(worker.merge.tier, Tier::Code);
    assert_eq!(worker.merge.rule, "deploys");
    assert_eq!(worker.authorize.tier, Tier::Owner);
    assert!(
        worker.authorize.rule.starts_with("secrets"),
        "a scope over backplane's crate reaches its secret handling: {}",
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
            adds_secret: false,
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
                adds_secret: false,
            },
        );
        assert!(tiers.authorize.tier >= Tier::Code, "{file}");
        assert!(tiers.merge.tier >= Tier::Code, "{file}");
    }
}

#[test]
fn an_operation_needs_the_tier_its_head_was_given_and_code_without_one() {
    use backplane::changes::{Change, State};
    let mut c = Change::propose(
        9,
        "x",
        "feat: x",
        "plan",
        vec!["tooling/**".into()],
        "agent",
    );
    c.state = State::Checked;
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.approve"),
        Tier::Code,
        "no tiers yet"
    );
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.close"),
        Tier::Confirm
    );
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.authorize"),
        Tier::Code
    );
    c.tiers = Some(tiers_of(&["tooling/**"], &["tooling/README.md"], PLAIN));
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.approve"),
        Tier::Confirm
    );
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.merge"),
        Tier::Confirm
    );
    let adr = [".claude/decisions/2026-11-07-development-job-images-on-cloudflare.md"];
    c.tiers = Some(tiers_of(
        &adr,
        &adr,
        Facts {
            deploys_worker: false,
            appends_only: true,
            adds_secret: false,
        },
    ));
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.merge"),
        Tier::Confirm,
        "auto counts as confirm until automatic approval lands"
    );
    c.tiers = Some(tiers_of(
        &["src/pigeon-cli/Dockerfile"],
        &["src/pigeon-cli/Dockerfile"],
        PLAIN,
    ));
    assert_eq!(
        backplane::approvals::tier_of(&c, "change.merge"),
        Tier::Code
    );
}

#[test]
fn the_control_plane_approves_and_merges_only_what_is_auto_or_confirm() {
    use backplane::approvals::{Tiered, by_policy};
    use backplane::changes::Change;
    let mut c = Change::propose(
        9,
        "x",
        "feat: x",
        "plan",
        vec!["tooling/**".into()],
        "agent",
    );
    assert_eq!(by_policy(&c, "change.approve"), None, "no tiers yet");
    c.tiers = Some(tiers_of(&["tooling/**"], &["tooling/README.md"], PLAIN));
    assert_eq!(by_policy(&c, "change.approve").as_deref(), Some("routine"));
    assert_eq!(by_policy(&c, "change.merge").as_deref(), Some("routine"));
    assert_eq!(
        by_policy(&c, "change.authorize"),
        None,
        "authorizing stays the owner's"
    );
    assert_eq!(
        by_policy(&c, "change.close").as_deref(),
        Some("unpushed"),
        "nothing pushed, nothing lost"
    );
    c.tiers = Some(tiers_of(
        &["src/pigeon-cli/Dockerfile"],
        &["src/pigeon-cli/Dockerfile"],
        PLAIN,
    ));
    assert_eq!(by_policy(&c, "change.merge"), None, "code is the owner's");
    let mut owner = tiers_of(&["tooling/**"], &["tooling/README.md"], PLAIN);
    owner.merge = Tiered {
        tier: Tier::Owner,
        rule: "secrets: x".into(),
    };
    c.tiers = Some(owner);
    assert_eq!(by_policy(&c, "change.merge"), None);
    assert!(Tier::Owner > Tier::Code, "owner outranks code");
}

#[test]
fn secrets_and_destruction_need_the_owner_each_time() {
    use backplane::approvals::{by_policy, lengthens, tier_of};
    use backplane::changes::Change;
    let t = tiers_of(&["infra/x/**"], &["infra/x/secrets.config.yaml"], PLAIN);
    assert_eq!(t.merge.tier, Tier::Owner, "{t:?}");
    let t = tiers_of(&[".mcp.json"], &[".mcp.json"], PLAIN);
    assert_eq!(t.authorize.tier, Tier::Owner, "{t:?}");
    let rules = tiers_of(
        &[".config/approvals.yaml"],
        &[".config/approvals.yaml"],
        PLAIN,
    );
    assert_eq!(rules.merge.tier, Tier::Owner, "the rules themselves");
    let lengthened = tiers_of(
        &["infra/pigeon/workers/control/worker.yaml"],
        &["infra/pigeon/workers/control/worker.yaml"],
        Facts {
            deploys_worker: true,
            appends_only: false,
            adds_secret: true,
        },
    );
    assert_eq!(
        (lengthened.merge.tier, lengthened.merge.rule.as_str()),
        (Tier::Owner, "adds a secret")
    );
    assert!(lengthens(&["A".into()], &["A".into(), "B".into()]));
    assert!(
        !lengthens(&["A".into(), "B".into()], &["A".into()]),
        "removing one doesn't"
    );
    let mut c = Change::propose(
        9,
        "x",
        "feat: x",
        "plan",
        vec!["tooling/**".into()],
        "agent",
    );
    assert_eq!(by_policy(&c, "change.close").as_deref(), Some("unpushed"));
    c.head = Some("abc".into());
    assert_eq!(by_policy(&c, "change.close"), None);
    assert_eq!(
        tier_of(&c, "change.close"),
        Tier::Owner,
        "a pushed change's work would be lost"
    );
}
