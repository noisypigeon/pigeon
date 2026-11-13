//! The approval transitions as tables: what each answers, and the state a refusal keeps.

use super::grants::Grants;
use super::mandates::{Mandates, State};
use super::passkeys::{Keyring, REGISTER};
use super::totp::{Guard, STEP, code};
use crate::core::change::{Change, State as ChangeState};

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
    use super::super::windows::{BUDGET, Windows};
    use crate::core::change::Change;

    const NOW: u64 = 1_800_000_000;

    fn change(merge: Tier) -> Change {
        let mut c = Change::propose(5, "x", "feat: x", "p", vec!["a/**".into()], "agent");
        let t = |tier| Tiered {
            tier,
            rule: "rule".into(),
        };
        c.tiers = Some(Tiers {
            authorize: t(Tier::Code),
            merge: t(merge),
            close: t(Tier::Confirm),
        });
        c
    }

    #[test]
    fn a_window_starts_for_a_set_length_and_one_at_a_time() {
        let (w, first) = Windows::default().start(4, "owner", NOW).expect("started");
        assert_eq!(first.expires, NOW + 4 * 3_600);
        assert!(w.start(4, "owner", NOW).is_err(), "one at a time");
        assert!(
            Windows::default().start(3, "owner", NOW).is_err(),
            "1, 4, 8 or 24"
        );
        let (w, ended) = w.end("owner ended it").expect("ended");
        assert_eq!(ended.state, State::Ended);
        assert!(w.current().is_none());
        assert!(w.end("again").is_err());
    }

    #[test]
    fn auto_lets_the_code_tier_through_and_asks_for_the_owner_tier() {
        let (w, _) = Windows::default().start(1, "owner", NOW).expect("started");
        for (tier, allowed) in [
            (Tier::Confirm, true),
            (Tier::Code, true),
            (Tier::Owner, false),
        ] {
            let (_, d) = w.decide(&change(tier), "change.merge", NOW + 10);
            assert_eq!(
                d.map(|(_, d)| d == Decision::Allow),
                Some(allowed),
                "{tier:?}"
            );
        }
        let (_, d) = w.decide(&change(Tier::Code), crate::control::BREAK_GLASS, NOW);
        assert!(matches!(d, Some((_, Decision::Ask(_)))), "break-glass asks");
        let (_, d) = w.decide(&change(Tier::Code), "change.merge", NOW + 3_600);
        assert_eq!(d, None, "the window has run its time");
        let (_, none) = Windows::default().decide(&change(Tier::Code), "change.merge", NOW);
        assert_eq!(none, None, "no window, no decision");
    }

    #[test]
    fn merges_count_against_the_budget_and_a_pause_stops_it() {
        let (mut w, _) = Windows::default().start(24, "owner", NOW).expect("started");
        for _ in 0..BUDGET {
            let (next, d) = w.decide(&change(Tier::Code), "change.merge", NOW);
            assert!(matches!(d, Some((_, Decision::Allow))));
            w = next;
        }
        assert_eq!(w.list[0].state, State::Ended, "the budget is used");
        assert_eq!(u32::try_from(w.list[0].merged.len()), Ok(BUDGET));

        let (w, _) = Windows::default().start(1, "owner", NOW).expect("started");
        let (w, paused) = w.pause_all("the deploy of abc failed");
        assert_eq!(paused, Some(1));
        let (_, d) = w.decide(&change(Tier::Code), "change.merge", NOW);
        assert_eq!(d, None, "a paused window approves nothing");
        assert!(w.current().is_some(), "it waits for the owner to end it");
        let (w, _) = w
            .start(1, "owner", NOW)
            .expect("a new window ends the paused one");
        assert_eq!(w.list[0].state, State::Ended);
    }
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
        expires: 5,
        ..window
    };
    assert_ne!(
        decide(&ended, &landed, "change.close", 10),
        Decision::Allow,
        "an ended window"
    );
}
