//! Approval tiers from `.config/approvals.yaml`; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md`.

use agent::core::approval::tiers::{
    AUTO_COVERS, Facts, Rules, Tier, Tiers, classify, matches, mixed_with_eviction,
};

fn rules() -> Rules {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/approvals.yaml");
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
        "src/pigeon-agent/migrations/0001.sql"
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
    let dockerfile = ["Dockerfile"];
    let tiers = tiers_of(&dockerfile, &dockerfile, PLAIN);
    assert_eq!(tiers.authorize.tier, Tier::Code);
    assert_eq!(tiers.merge.tier, Tier::Code);
    let worker = tiers_of(
        &["src/pigeon-agent/**"],
        &["src/pigeon-agent/src/ship.rs"],
        Facts {
            deploys_worker: true,
            appends_only: false,
            adds_secret: false,
        },
    );
    assert_eq!(worker.merge.tier, Tier::Code);
    assert_eq!(worker.merge.rule, "deploys");
    assert_eq!(worker.authorize.tier, Tier::Code, "the merge decides");
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
fn the_approval_core_needs_the_code_as_the_files_it_moved_from_did() {
    for name in ["tiers", "grants", "mandates", "passkeys", "totp"] {
        let file = format!("src/pigeon-core/src/approval/{name}.rs");
        let tiers = tiers_of(
            &[file.as_str()],
            &[file.as_str()],
            Facts {
                deploys_worker: false,
                appends_only: false,
                adds_secret: false,
            },
        );
        assert!(tiers.authorize.tier >= Tier::Code, "{file}");
        assert!(tiers.merge.tier >= Tier::Code, "{file}");
    }
}

#[test]
fn an_operation_needs_the_tier_its_head_was_given_and_code_without_one() {
    use agent::core::change::{Change, State};
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
        agent::core::approval::tiers::tier_of(&c, "change.approve"),
        Tier::Code,
        "no tiers yet"
    );
    assert_eq!(
        agent::core::approval::tiers::tier_of(&c, "change.close"),
        Tier::Confirm
    );
    assert_eq!(
        agent::core::approval::tiers::tier_of(&c, "change.authorize"),
        Tier::Code
    );
    c.tiers = Some(tiers_of(&["tooling/**"], &["tooling/README.md"], PLAIN));
    assert_eq!(
        agent::core::approval::tiers::tier_of(&c, "change.approve"),
        Tier::Confirm
    );
    assert_eq!(
        agent::core::approval::tiers::tier_of(&c, "change.merge"),
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
        agent::core::approval::tiers::tier_of(&c, "change.merge"),
        Tier::Confirm,
        "auto counts as confirm until automatic approval lands"
    );
    c.tiers = Some(tiers_of(&["Dockerfile"], &["Dockerfile"], PLAIN));
    assert_eq!(
        agent::core::approval::tiers::tier_of(&c, "change.merge"),
        Tier::Code
    );
}

#[test]
fn the_control_plane_approves_and_merges_only_what_is_auto_or_confirm() {
    use agent::core::approval::tiers::{Tiered, by_policy};
    use agent::core::change::Change;
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
    c.tiers = Some(tiers_of(&["Dockerfile"], &["Dockerfile"], PLAIN));
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
    use agent::core::approval::tiers::{by_policy, lengthens, tier_of};
    use agent::core::change::Change;
    let t = tiers_of(&["infra/x/**"], &["infra/x/secrets.config.yaml"], PLAIN);
    assert_eq!(t.merge.tier, Tier::Owner, "{t:?}");
    let t = tiers_of(&[".mcp.json"], &[".mcp.json"], PLAIN);
    assert_eq!(t.authorize.tier, Tier::Code, "{t:?}");
    assert!(t.authorize.rule.ends_with("(the merge decides)"), "{t:?}");
    assert_eq!(t.merge.tier, Tier::Owner, "{t:?}");
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

#[test]
fn one_proof_covers_a_changes_later_stages_up_to_the_code_tier() {
    use agent::core::approval::tiers::by_grant;
    use agent::core::change::{Change, ChangeGrant};
    let mut c = Change::propose(9, "x", "feat: x", "plan", vec!["infra/**".into()], "agent");
    assert_eq!(by_grant(&c, "change.authorize", 100), None, "no proof yet");
    c.grant = Some(ChangeGrant {
        by: "owner@example.com".into(),
        expires: 1000,
        review: false,
    });
    assert_eq!(
        by_grant(&c, "change.authorize", 100).as_deref(),
        Some("owner@example.com")
    );
    c.head = Some("abc".into());
    c.tiers = Some(tiers_of(&["infra/**"], &["infra/x/unit.yaml"], PLAIN));
    assert_eq!(c.tiers.as_ref().map(|t| t.merge.tier), Some(Tier::Code));
    assert!(
        by_grant(&c, "change.merge", 100).is_some(),
        "a deploying head"
    );
    assert_eq!(by_grant(&c, "change.merge", 1000), None, "lapsed");
    assert_eq!(
        by_grant(&c, "change.close", 100),
        None,
        "closing is never covered"
    );
    c.tiers = Some(tiers_of(
        &["infra/**"],
        &["infra/x/secrets.config.yaml"],
        PLAIN,
    ));
    assert_eq!(
        by_grant(&c, "change.merge", 100),
        None,
        "a secret asks again"
    );
    c.tiers = Some(tiers_of(&["infra/**"], &["infra/x/unit.yaml"], PLAIN));
    c.grant.as_mut().expect("grant").review = true;
    assert!(by_grant(&c, "change.approve", 100).is_some());
    assert_eq!(
        by_grant(&c, "change.merge", 100),
        None,
        "the owner kept the merge"
    );
}

#[test]
fn the_owners_approval_of_a_head_covers_its_merge_at_the_owner_tier() {
    // !800, 2026-11-16: approved by proof at 1794860506, `change.merge` asked again at
    // 1794860519; see the approvals ADR's 2026-11-16 amendment.
    use crate::harness::step::{Apply, Queue};
    use agent::core::approval::grants::PAGE_TTL;
    use agent::core::approval::tiers::by_grant;
    use agent::core::change::queue::{MergeQueue, Queued};
    use agent::core::change::{Change, ChangeGrant, Event, State};
    const OWNER: &str = "acct_owner";
    const HEAD: &str = "8000000000000000000000000000000000000800";
    let (approved, merged) = (1_794_860_506, 1_794_860_519);
    let mut c = Change::propose(800, "x", "feat: x", "plan", vec!["**".into()], "agent");
    c.state = State::Checked;
    c.head = Some(HEAD.into());
    c.tiers = Some(tiers_of(
        &[".config/**"],
        &[".config/approvals.yaml"],
        PLAIN,
    ));
    assert_eq!(c.tiers.as_ref().map(|t| t.merge.tier), Some(Tier::Owner));
    c.grant = Some(ChangeGrant {
        by: OWNER.into(),
        expires: approved + PAGE_TTL,
        review: false,
    });
    assert_eq!(
        by_grant(&c, "change.approve", approved),
        None,
        "approving the owner tier takes its own proof"
    );
    assert_eq!(
        by_grant(&c, "change.merge", approved),
        None,
        "not approved yet"
    );
    c.apply(Event::Approve {
        by: OWNER.into(),
        sha: HEAD.into(),
        at: approved,
    })
    .expect("approved");
    let mut q = MergeQueue::default();
    q.enqueue(799, "owner", approved - 60)
        .expect("another merges");
    assert_eq!(q.ask(800, OWNER, approved).1, Ok(Queued::Behind(1)));

    assert_eq!(
        by_grant(&c, "change.merge", merged).as_deref(),
        Some(OWNER),
        "the session's merge files no ask"
    );
    let place = q.ask(800, &format!("grant:{OWNER}"), merged).1;
    assert_eq!(
        place,
        Err("change 800 already waits behind 1 merge(s)".into()),
        "it answers the queue's place"
    );

    let mut held = c.clone();
    held.grant.as_mut().expect("grant").review = true;
    assert_eq!(
        by_grant(&held, "change.merge", merged),
        None,
        "a hold holds"
    );
    assert_eq!(
        by_grant(&c, "change.merge", approved + PAGE_TTL),
        None,
        "a day-old approval asks again"
    );
    let mut moved = c.clone();
    moved.head = Some("8010000000000000000000000000000000000801".into());
    assert_eq!(by_grant(&moved, "change.merge", merged), None, "a new head");
    let mut other = c.clone();
    other.approval.as_mut().expect("approval").by = "window:1".into();
    assert_eq!(
        by_grant(&other, "change.merge", merged),
        None,
        "only the prover's own approval"
    );
}

/// This repository's rules, with sentinel's paths named.
fn with_sentinel() -> Rules {
    let mut r = rules();
    r.eviction = paths(&[
        "src/pigeon-eviction/**",
        ".config/policies/**",
        ".config/approvals.yaml",
    ]);
    r
}

#[test]
fn a_sentinel_change_needs_the_code_and_says_why() {
    let r = with_sentinel();
    let file = "src/pigeon-eviction/src/rules.rs";
    let tiers = classify(
        &r,
        &paths(&["src/pigeon-eviction/**"]),
        &paths(&[file]),
        PLAIN,
    );
    assert_eq!(tiers.authorize.tier, Tier::Code);
    assert!(tiers.authorize.rule.starts_with("eviction"), "{tiers:?}");
    assert_eq!(tiers.merge.tier, Tier::Code);
    let mark = if r.gate.is_some() { AUTO_COVERS } else { "" };
    assert_eq!(tiers.merge.rule, format!("eviction: {file}{mark}"));
    let gate = "src/pigeon-core/src/approval/tiers.rs";
    let gated = classify(&r, &paths(&[gate]), &paths(&[gate]), PLAIN);
    assert_eq!(
        gated.merge.rule,
        format!("guarded: {gate}"),
        "the gate stays plain"
    );
    let rules_file = classify(
        &r,
        &paths(&[".config/approvals.yaml"]),
        &paths(&[".config/approvals.yaml"]),
        PLAIN,
    );
    assert_eq!(
        rules_file.merge.tier,
        Tier::Owner,
        "a secret path stays the owner's"
    );
}

#[test]
fn a_sentinel_change_holds_nothing_but_sentinel_and_its_adr() {
    let r = with_sentinel();
    let alone = paths(&[
        "src/pigeon-eviction/src/rules.rs",
        "src/pigeon-eviction/tests/cli.rs",
        ".claude/decisions/2026-11-10-x.md",
    ]);
    assert_eq!(mixed_with_eviction(&r, &alone), None);
    let mixed = paths(&[
        ".config/policies/change.yaml",
        "src/pigeon-agent/src/jobs.rs",
    ]);
    let why = mixed_with_eviction(&r, &mixed).expect("refused");
    assert!(why.contains("src/pigeon-agent/src/jobs.rs"), "{why}");
    assert_eq!(
        mixed_with_eviction(&r, &paths(&["src/pigeon-agent/src/jobs.rs"])),
        None,
        "no sentinel path, nothing to keep apart"
    );
    let moved = paths(&[
        "src/pigeon-eviction/src/rules.rs",
        "Cargo.toml",
        "Cargo.lock",
    ]);
    let mut bare = r.clone();
    bare.eviction_bookkeeping.clear();
    let why = mixed_with_eviction(&bare, &moved).expect("no bookkeeping listed");
    assert!(why.contains("Cargo.toml"), "{why}");
    let mut kept = r.clone();
    kept.eviction_bookkeeping = paths(&["Cargo.toml", "Cargo.lock"]);
    assert_eq!(
        mixed_with_eviction(&kept, &moved),
        None,
        "a sentinel change may carry the workspace's bookkeeping"
    );
    assert!(
        mixed_with_eviction(&kept, &mixed).is_some(),
        "but nothing else"
    );
    assert_eq!(
        mixed_with_eviction(&rules(), &mixed).is_some(),
        !rules().eviction.is_empty(),
        "without a sentinel list, nothing is kept apart"
    );
}

#[test]
fn the_break_glass_is_the_owners_alone() {
    use agent::control::BREAK_GLASS;
    use agent::core::approval::tiers::{by_grant, by_policy, tier_of};
    use agent::core::change::{Change, ChangeGrant};
    let mut c = Change::propose(9, "x", "feat: x", "plan", vec!["src/**".into()], "agent");
    c.head = Some("abc".into());
    c.grant = Some(ChangeGrant {
        by: "owner@example.com".into(),
        expires: 1000,
        review: false,
    });
    assert_eq!(tier_of(&c, BREAK_GLASS), Tier::Owner);
    assert_eq!(
        by_grant(&c, BREAK_GLASS, 100),
        None,
        "the change's one proof doesn't cover it"
    );
    assert_eq!(by_policy(&c, BREAK_GLASS), None);
}
