//! The approval transitions as tables: what each answers, and the state a refusal keeps.

use super::grants::Grants;
use super::mandates::{Mandates, State};
use super::passkeys::{Keyring, REGISTER};
use super::totp::{Guard, STEP, code};
use crate::change::{Change, State as ChangeState};

const SECRET: &[u8] = b"12345678901234567890";
const HEAD: &str = "1111111111111111111111111111111111111111";
const ADR: &str = ".claude/decisions/2026-11-10-development-factory-crates-functional-core.md";

fn checked() -> Change {
    let mut c = Change::propose(1, "x", "feat: x", "p", vec!["**".into()], "agent");
    c.state = ChangeState::Checked;
    c.head = Some(HEAD.into());
    c
}

#[test]
fn a_code_answers_the_next_guard_and_a_wrong_one_counts_while_the_old_guard_stays() {
    let now = 1_000 * STEP;
    let right = format!("{:06}", code(SECRET, now / STEP));
    let table: Vec<(&str, &str, bool, usize)> = vec![
        ("the right code", &right, true, 0),
        ("a wrong code", "000000", false, 1),
        ("no code", "abc", false, 1),
    ];
    for (name, typed, ok, failures) in table {
        let before = Guard::default();
        let (next, checked) = before.verify(SECRET, typed, now);
        assert_eq!(checked.is_ok(), ok, "{name}");
        assert_eq!(next.failures.len(), failures, "{name}");
        assert_eq!(
            before,
            Guard::default(),
            "{name}: the old guard is unchanged"
        );
    }
}

#[test]
fn a_redeemed_grant_is_spent_even_when_refused() {
    let c = checked();
    let (grants, minted) = Grants::default().mint(&c, "change.approve", "tok", 10);
    minted.expect("mints");
    let (spent, refused) = grants.redeem("tok", "change.merge", &c, 11);
    assert!(refused.is_err(), "for another operation");
    assert!(
        grants.peek("tok", 1, 11).is_ok(),
        "the old grants still hold it"
    );
    assert!(
        spent.peek("tok", 1, 11).is_err(),
        "the next ones hold it spent"
    );
    let (_, again) = spent.redeem("tok", "change.approve", &c, 12);
    assert!(again.is_err(), "once only");
}

#[test]
fn a_mandates_grant_is_spent_even_when_refused() {
    let (all, proposed) =
        Mandates::default().propose(ADR, "task", vec!["src/**".into()], false, None, None);
    proposed.expect("proposes");
    let (all, offered) = all.offer(1, "tok", 0, 100);
    offered.expect("offers");
    let (all, ended) = all.end(1, "stop");
    assert_eq!(ended.expect("ends").state, State::Ended);
    let (spent, refused) = all.grant("tok", "owner", 1);
    assert!(refused.is_err(), "an ended mandate isn't granted");
    assert!(
        all.peek("tok", 1).is_ok(),
        "the old mandates still hold the grant"
    );
    assert!(spent.peek("tok", 1).is_err(), "the next ones hold it spent");
}

#[test]
fn a_registration_uses_its_challenge_up_even_when_refused() {
    let (ring, challenge) = Keyring::default().mint(REGISTER, &[7u8; 32], 10);
    let client = format!(r#"{{"type":"webauthn.create","challenge":"{challenge}"}}"#);
    let (next, refused) = ring.register("id", "phone", client.as_bytes(), b"", b"", "h", "h", 11);
    assert!(refused.is_err(), "no authenticator data");
    assert_eq!(ring.challenges.len(), 1, "the old keyring keeps it");
    assert!(next.challenges.is_empty(), "the next one has used it up");
    let (unknown, refused) = ring.register("id", "phone", b"{}", b"", b"", "h", "h", 11);
    assert!(refused.is_err(), "no challenge answered");
    assert_eq!(
        unknown, ring,
        "a refusal before the challenge keeps the keyring"
    );
}

mod windows {
    use super::super::mandates::{Decision, State};
    use super::super::tiers::{Tier, Tiered, Tiers};
    use super::super::windows::{QUIET, RATE, SPAN, Windows};
    use crate::change::Change;

    const NOW: u64 = 1_800_000_000;

    fn numbered(n: u32, merge: Tier, rule: &str) -> Change {
        let mut c = Change::propose(n, "x", "feat: x", "p", vec!["a/**".into()], "agent");
        let t = |tier, rule: &str| Tiered {
            tier,
            rule: rule.into(),
        };
        c.tiers = Some(Tiers {
            authorize: t(Tier::Code, "rule"),
            merge: t(merge, rule),
            close: t(Tier::Confirm, "rule"),
        });
        c
    }

    fn change(merge: Tier) -> Change {
        numbered(5, merge, "rule")
    }

    #[test]
    fn auto_is_standing_and_one_runs_at_a_time() {
        let (w, first) = Windows::default().start("owner", NOW).expect("started");
        assert_eq!((first.hours, first.expires), (0, NOW));
        assert!(w.start("owner", NOW).is_err(), "one at a time");
        let year = NOW + 365 * 24 * 3_600;
        let (_, d) = w.decide(&change(Tier::Code), "change.merge", year);
        assert_eq!(d, Some((1, Decision::Allow)), "it never expires");
        let (w, ended) = w.end("the owner paused it").expect("ended");
        assert_eq!(ended.state, State::Ended);
        assert!(w.current().is_none());
        assert!(w.end("again").is_err());
    }

    #[test]
    fn auto_lets_the_code_tier_through_but_not_guarded_eviction_or_owner() {
        let (w, _) = Windows::default().start("owner", NOW).expect("started");
        for (tier, rule, allowed) in [
            (Tier::Confirm, "rule", true),
            (Tier::Code, "deploys", true),
            (Tier::Code, "guarded: .mcp.json", false),
            (Tier::Code, "sentinel: .config/policies/**", false),
            (Tier::Owner, "secrets: x", false),
        ] {
            for op in ["change.approve", "change.merge"] {
                let (_, d) = w.decide(&numbered(5, tier, rule), op, NOW + 10);
                assert_eq!(
                    d.map(|(_, d)| d == Decision::Allow),
                    Some(allowed),
                    "{op} {rule}"
                );
            }
        }
        let guarded = numbered(5, Tier::Code, "guarded: .mcp.json");
        let (_, d) = w.decide(&guarded, "change.authorize", NOW);
        assert_eq!(d, Some((1, Decision::Allow)), "its plan is still covered");
        let (_, d) = w.decide(&change(Tier::Code), crate::control::BREAK_GLASS, NOW);
        assert!(matches!(d, Some((_, Decision::Ask(_)))), "break-glass asks");
        let (_, none) = Windows::default().decide(&change(Tier::Code), "change.merge", NOW);
        assert_eq!(none, None, "no window, no decision");
    }

    #[test]
    fn every_merge_counts_once_however_it_was_asked_for() {
        let (w, _) = Windows::default().start("owner", NOW).expect("started");
        let (w, _) = w.decide(&numbered(1, Tier::Code, "rule"), "change.approve", NOW);
        let (w, _) = w.decide(&numbered(1, Tier::Code, "rule"), "change.approve", NOW);
        let (w, _) = w.decide(&numbered(1, Tier::Code, "rule"), "change.merge", NOW);
        let (w, _) = w.decide(&numbered(2, Tier::Code, "rule"), "change.merge", NOW);
        let (w, _) = w.decide(&numbered(3, Tier::Code, "rule"), "change.authorize", NOW);
        let (w, _) = w.decide(&numbered(4, Tier::Owner, "rule"), "change.approve", NOW);
        assert_eq!(w.list[0].merged, vec![1, 2], "approved-and-queued counts");
        assert_eq!((w.list[0].used, w.list[0].rate(NOW)), (2, 2));
        let (held, _) = w.hold_all("abc", "token", NOW);
        let (held, _) = held.decide(&numbered(6, Tier::Code, "rule"), "change.approve", NOW);
        assert_eq!(
            held.list[0].merged,
            vec![1, 2],
            "a held approval doesn't queue"
        );
    }

    #[test]
    fn the_rate_breaker_holds_merges_and_releases_after_a_quiet_hour() {
        let (mut w, _) = Windows::default().start("owner", NOW).expect("started");
        for n in 0..u32::try_from(RATE).expect("small") {
            let (next, d) = w.decide(&numbered(n, Tier::Code, "rule"), "change.approve", NOW);
            assert_eq!(d, Some((1, Decision::Allow)), "{n}");
            w = next;
        }
        assert_eq!((w.list[0].state, w.list[0].quiet), (State::Held, Some(NOW)));
        assert!(
            w.list[0]
                .why
                .as_deref()
                .is_some_and(|y| y.contains("150 merges"))
        );
        let late = NOW + 1_800;
        let (w, d) = w.decide(&titled("feat: x"), "change.merge", late);
        assert!(matches!(d, Some((1, Decision::Hold(_)))), "{d:?}");
        let (w, d) = w.decide(&numbered(900, Tier::Code, "rule"), "change.approve", late);
        assert_eq!(d, Some((1, Decision::Allow)), "approvals still land");
        assert!(
            !w.list[0].merged.contains(&900),
            "a held approval isn't counted"
        );
        let fix = Change {
            number: 901,
            ..titled("fix: x")
        };
        let (w, d) = w.decide(&fix, "change.merge", late);
        assert_eq!(
            d,
            Some((1, Decision::Allow)),
            "a fix goes through, and is counted"
        );
        let (still, _) = w.resume_all(late);
        assert_eq!(
            still.list[0].state,
            State::Held,
            "a passed deploy doesn't release it"
        );
        let quiet = late + QUIET;
        let released = w.settle(quiet);
        assert_eq!(
            (
                released.list[0].state,
                released.list[0].quiet,
                released.list[0].rate(quiet)
            ),
            (State::Active, None, 0),
            "an hour with no counted merge releases it, with a fresh count"
        );
        assert_eq!(super::super::windows::moved(&w, &released).len(), 1);
        let (_, d) = released.decide(&titled("feat: x"), "change.merge", quiet);
        assert_eq!(d, Some((1, Decision::Allow)));
        assert_eq!(
            w.settle(quiet - 60).list[0].state,
            State::Held,
            "not before the hour"
        );

        let (mut w, _) = Windows::default().start("owner", NOW).expect("started");
        for n in 0..u32::try_from(RATE).expect("small") {
            let at = NOW + u64::from(n) * (SPAN / 40);
            w = w
                .decide(&numbered(n, Tier::Code, "rule"), "change.merge", at)
                .0;
        }
        assert_eq!(w.list[0].state, State::Active, "spread out, it runs on");
    }

    fn titled(title: &str) -> Change {
        Change {
            title: title.into(),
            ..change(Tier::Code)
        }
    }

    #[test]
    fn a_failed_deploy_holds_merges_but_a_fix_and_a_passed_deploy_resumes_it() {
        let (w, _) = Windows::default().start("owner", NOW).expect("started");
        let (w, held) = w.hold_all("abc", "token", NOW + 600);
        assert_eq!(held, Some((1, false)));
        let late = NOW + 2 * 3_600;
        for op in ["change.authorize", "change.approve", "change.close"] {
            let (_, d) = w.decide(&titled("feat: x"), op, late);
            assert_eq!(d, Some((1, Decision::Allow)), "{op}");
        }
        let (_, d) = w.decide(&titled("feat: x"), "change.merge", late);
        assert!(matches!(d, Some((1, Decision::Hold(_)))), "{d:?}");
        let (_, d) = w.decide(&change(Tier::Owner), "change.merge", late);
        assert!(
            matches!(d, Some((1, Decision::Ask(_)))),
            "the owner tier still asks: {d:?}"
        );
        for fix in ["fix: x", "fix(infra): x", "revert: x"] {
            let (_, d) = w.decide(&titled(fix), "change.merge", late);
            assert_eq!(d, Some((1, Decision::Allow)), "{fix}");
        }
        let (w, _) = w.decide(&titled("fix: x"), "change.merge", late);
        let (w, resumed) = w.resume_all(late);
        let r = resumed.expect("resumed");
        assert_eq!((r.state, r.hold), (State::Active, None));
        assert_eq!(r.used, 1, "its merges are kept");
        let (_, d) = w.decide(&numbered(9, Tier::Code, "rule"), "change.merge", late + 60);
        assert_eq!(d, Some((1, Decision::Allow)));
    }

    #[test]
    fn a_hold_pauses_auto_after_three_failed_deploys_or_six_hours() {
        let (w, _) = Windows::default().start("owner", NOW).expect("started");
        let (w, _) = w.hold_all("a", "x", NOW);
        let (w, again) = w.hold_all("b", "y", NOW + 60);
        assert_eq!(again, Some((1, false)));
        assert_eq!(
            w.list[0].hold.as_ref().map(|h| (h.failures, h.since)),
            Some((2, NOW))
        );
        let (paused, third) = w.hold_all("c", "z", NOW + 120);
        assert_eq!(third, Some((1, true)));
        assert_eq!(paused.list[0].state, State::Paused);
        let timed = w.settle(NOW + super::super::mandates::HOLD_MAX);
        assert_eq!(timed.list[0].state, State::Paused);
        assert!(timed.current().is_some(), "it waits for the owner");
        let (_, none) = timed.resume_all(NOW + 7 * 3_600);
        assert!(none.is_none(), "a paused window never resumes by itself");
        let (w, _) = w
            .start("owner", NOW + 180)
            .expect("the owner's start ends the held one");
        assert_eq!(w.list[0].state, State::Ended);
    }
}

#[test]
fn a_failed_deploy_holds_a_mandate_and_a_passed_one_resumes_it() {
    use super::mandates::{Decision, Mandates, State as M};
    use super::tiers::{Tier, Tiered, Tiers};
    let (all, m) = Mandates::default().propose(
        ".claude/decisions/x.md",
        "x",
        vec!["**".into()],
        false,
        None,
        Some(1),
    );
    let mut all = all;
    let m = m.expect("proposed");
    (all.list[0].state, all.list[0].expires) = (M::Active, Some(100));
    let (paused, _) = all.pause_all("scope");
    let (held, ids, ended) = all.hold_all("abc", "token", 50);
    assert_eq!((ids, ended), (vec![m.id], Vec::new()));
    let mut c = checked();
    (c.plan, c.mandate) = (".claude/decisions/x.md".into(), Some(m.id));
    let t = |tier| Tiered {
        tier,
        rule: "rule".into(),
    };
    c.tiers = Some(Tiers {
        authorize: t(Tier::Confirm),
        merge: t(Tier::Confirm),
        close: t(Tier::Confirm),
    });
    let (_, d) = held.decide(&c, "change.authorize", 500);
    assert!(matches!(d, Some((_, Decision::Allow))), "{d:?}");
    let (_, d) = held.decide(&c, "change.merge", 500);
    assert!(matches!(d, Some((_, Decision::Hold(_)))), "{d:?}");
    let (resumed, ids) = held.resume_all(500);
    assert_eq!(ids, vec![m.id]);
    assert_eq!(
        resumed.list[0].expires,
        Some(550),
        "expiry moves by the hold"
    );
    assert_eq!(resumed.list[0].state, M::Active);
    let (p2, _, _) = paused.hold_all("abc", "token", 50);
    assert_eq!(p2.list[0].state, M::Paused, "a pause stays the owner's");
    let (spent, ended) = held.settle(50 + super::mandates::HOLD_MAX);
    assert_eq!((spent.list[0].state, ended), (M::Ended, vec![m.id]));
}

#[test]
fn closing_a_landed_head_is_code_and_an_auto_window_approves_it_but_not_another_pushed_close() {
    use super::mandates::Decision;
    use super::tiers::{Tier, tier_of};
    use super::windows::{Window, decide};
    let window = Window {
        id: 3,
        hours: 4,
        started: 0,
        expires: 100,
        used: 0,
        state: State::Active,
        by: "owner".into(),
        why: None,
        merged: Vec::new(),
        hold: None,
        recent: Vec::new(),
        quiet: None,
    };
    let pushed = checked();
    let mut landed = checked();
    landed.landed = Some(HEAD.into());
    let mut pushed_since = landed.clone();
    pushed_since.head = Some("2222222222222222222222222222222222222222".into());
    let unpushed = Change::propose(2, "y", "feat: y", "p", vec!["**".into()], "agent");
    let table: Vec<(&str, &Change, Tier, bool)> = vec![
        (
            "a pushed change throws its work away",
            &pushed,
            Tier::Owner,
            false,
        ),
        ("its head is already on main", &landed, Tier::Code, true),
        (
            "a push since moved the head",
            &pushed_since,
            Tier::Owner,
            false,
        ),
        ("nothing was pushed", &unpushed, Tier::Confirm, true),
    ];
    for (name, c, tier, allowed) in table {
        assert_eq!(tier_of(c, "change.close"), tier, "{name}");
        let d = decide(&window, c, "change.close", 10);
        assert_eq!(d == Decision::Allow, allowed, "{name}: {d:?}");
    }
    let ended = Window {
        state: State::Ended,
        ..window
    };
    assert_ne!(
        decide(&ended, &landed, "change.close", 10),
        Decision::Allow,
        "an ended window"
    );
}

#[test]
fn a_pin_is_the_owners_each_time_and_offered_only_on_an_open_change() {
    use super::grants::offerable;
    use super::tiers::{Tier, tier_of};
    let c = checked();
    assert_eq!(tier_of(&c, "change.pin"), Tier::Owner);
    assert!(offerable(&c, "change.pin").is_ok());
    let mut closed = checked();
    closed.state = ChangeState::Closed;
    assert!(offerable(&closed, "change.pin").is_err());
}

#[test]
fn a_move_to_the_front_is_the_owners_each_time_and_offered_only_on_an_open_change() {
    use super::grants::offerable;
    use super::tiers::{Tier, tier_of};
    let c = checked();
    assert_eq!(tier_of(&c, "change.front"), Tier::Owner);
    assert!(offerable(&c, "change.front").is_ok());
    let mut merged = checked();
    merged.state = ChangeState::Merged;
    assert!(offerable(&merged, "change.front").is_err());
}

#[test]
fn the_judges_rules_read_only_under_eviction() {
    use super::tiers::{Rules, is_judge_rule};
    let new: Rules = serde_saphyr::from_str(
        "guarded: []\neviction: [a/**]\neviction_bookkeeping: [Cargo.toml]\n",
    )
    .expect("new");
    assert_eq!(new.eviction, ["a/**"]);
    assert_eq!(new.eviction_bookkeeping, ["Cargo.toml"]);
    assert!(
        serde_saphyr::from_str::<Rules>("guarded: []\nsentinel: [a/**]\n").is_err(),
        "the old key is refused"
    );
    // A head classified before step 12 keeps its stored `sentinel:` label, which still counts.
    assert!(is_judge_rule("sentinel: a/b.rs"));
    assert!(is_judge_rule("eviction: a/b.rs"));
    assert!(!is_judge_rule("guarded: a/b.rs"));
}

#[test]
fn with_a_gate_auto_covers_guarded_and_eviction_paths_but_not_the_gate_or_secrets() {
    use super::mandates::Decision;
    use super::tiers::{AUTO_COVERS, Facts, Rules, Tier, classify};
    use super::windows::Windows;
    let yaml = "guarded: [.mcp.json, core/approval/**]\neviction: [.config/policies/**]\n\
                secrets: [secrets.config.yaml]\n";
    let ungated: Rules = serde_saphyr::from_str(yaml).expect("ungated");
    let gated: Rules =
        serde_saphyr::from_str(&format!("{yaml}gate: [core/approval/**]\n")).expect("gated");
    let (w, _) = Windows::default()
        .start("owner", 1_800_000_000)
        .expect("started");
    let decide = |rules: &Rules, file: &str| {
        let files = vec![file.to_string()];
        let mut c = Change::propose(5, "x", "feat: x", "p", files.clone(), "agent");
        c.tiers = Some(classify(rules, &files, &files, Facts::default()));
        let rule = c.tiers.as_ref().expect("tiers").merge.clone();
        let (_, d) = w.decide(&c, "change.merge", 1_800_000_010);
        (rule, d.map(|(_, d)| d == Decision::Allow))
    };
    for (file, covered) in [
        (".mcp.json", true),
        (".config/policies/x.yaml", true),
        ("core/approval/tiers.rs", false),
        ("secrets.config.yaml", false),
    ] {
        let (rule, allowed) = decide(&gated, file);
        assert_eq!(allowed, Some(covered), "{file}: {}", rule.rule);
        assert_eq!(rule.rule.ends_with(AUTO_COVERS), covered, "{file}");
        let (rule, allowed) = decide(&ungated, file);
        assert_eq!(allowed, Some(false), "no gate, no change: {file}");
        assert!(!rule.rule.ends_with(AUTO_COVERS));
    }
    assert_eq!(decide(&gated, "secrets.config.yaml").0.tier, Tier::Owner);
    let files = vec![".mcp.json".to_string(), "core/approval/x.rs".to_string()];
    let t = classify(&gated, &files, &files, Facts::default());
    assert_eq!(
        t.merge.rule, "guarded: core/approval/x.rs",
        "a gate file wins"
    );
}
