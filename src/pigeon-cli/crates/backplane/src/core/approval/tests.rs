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
