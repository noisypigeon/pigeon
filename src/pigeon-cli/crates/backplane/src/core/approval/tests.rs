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

    fn titled(title: &str) -> Change {
        Change {
            title: title.into(),
            ..change(Tier::Code)
        }
    }

    #[test]
    fn a_failed_deploy_holds_merges_but_a_fix_and_a_passed_deploy_resumes_it() {
        let (w, _) = Windows::default().start(1, "owner", NOW).expect("started");
        let (w, held) = w.hold_all("abc", "token", NOW + 600);
        assert_eq!(held, Some((1, false)));
        let late = NOW + 2 * 3_600;
        for op in ["change.authorize", "change.approve", "change.close"] {
            let (_, d) = w.decide(&titled("feat: x"), op, late);
            assert_eq!(d, Some((1, Decision::Allow)), "{op}: time stops while held");
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
        assert_eq!(r.state, State::Active);
        assert_eq!(
            r.expires,
            NOW + 3_600 + (late - NOW - 600),
            "the hold is added back"
        );
        assert_eq!(r.used, 1, "the budget is kept");
        let (_, d) = w.decide(&titled("feat: x"), "change.merge", late + 60);
        assert_eq!(d, Some((1, Decision::Allow)));
    }

    #[test]
    fn a_hold_ends_after_three_failed_deploys_or_six_hours() {
        let (w, _) = Windows::default().start(1, "owner", NOW).expect("started");
        let (w, _) = w.hold_all("a", "x", NOW);
        let (w, again) = w.hold_all("b", "y", NOW + 60);
        assert_eq!(again, Some((1, false)));
        assert_eq!(
            w.list[0].hold.as_ref().map(|h| (h.failures, h.since)),
            Some((2, NOW))
        );
        let (ended, third) = w.hold_all("c", "z", NOW + 120);
        assert_eq!(third, Some((1, true)));
        assert_eq!(ended.list[0].state, State::Ended);
        let timed = w.settle(NOW + super::super::mandates::HOLD_MAX);
        assert_eq!(timed.list[0].state, State::Ended);
        assert!(timed.current().is_none());
        let (_, none) = timed.resume_all(NOW + 7 * 3_600);
        assert!(none.is_none(), "an ended window never resumes");
        let (w, _) = w
            .start(4, "owner", NOW + 180)
            .expect("a new window ends the held one");
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
