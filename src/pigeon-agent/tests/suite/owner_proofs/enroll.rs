//! One-time enrolment; see
//! `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.

use agent::core::approval::enroll::{Enrolments, LOCK_AFTER, MAX_AHEAD, stored, token};

const NOW: u64 = 1_000_000;

#[test]
fn a_token_is_hex_and_its_stored_form_holds_only_its_digest() {
    let t = token(&[0xab; 32]);
    assert_eq!(t, "ab".repeat(32));
    let s = stored(&t, NOW + 600);
    let (expires, hash) = s.split_once('.').expect("two parts");
    assert_eq!(expires, (NOW + 600).to_string());
    assert_eq!(hash.len(), 64);
    assert!(!s.contains(&t), "the token isn't stored");
}

#[test]
fn an_enrolment_is_used_once_and_only_while_it_lasts() {
    let t = token(&[1; 32]);
    let s = stored(&t, NOW + 600);
    let mut all = Enrolments::default();
    assert!(kept!(all.redeem(None, &t, NOW)).is_err(), "no secret");
    assert!(kept!(all.redeem(Some(""), &t, NOW)).is_err(), "empty");
    assert!(
        kept!(all.redeem(Some("soon.abc"), &t, NOW)).is_err(),
        "malformed"
    );
    assert!(
        kept!(all.redeem(Some(&s), &t, NOW + 600)).is_err(),
        "expired"
    );
    let far = stored(&t, NOW + MAX_AHEAD + 1);
    assert!(
        kept!(all.redeem(Some(&far), &t, NOW)).is_err(),
        "too far ahead"
    );
    assert!(all.failures.is_empty(), "none of those was a wrong token");
    assert_eq!(kept!(all.redeem(Some(&s), &format!(" {t}\n"), NOW)), Ok(()));
    assert!(kept!(all.redeem(Some(&s), &t, NOW + 1)).is_err(), "used");
    let again = stored(&token(&[2; 32]), NOW + 900);
    assert_eq!(
        kept!(all.redeem(Some(&again), &token(&[2; 32]), NOW + 2)),
        Ok(())
    );
}

#[test]
fn five_wrong_tokens_lock_enrolment_for_the_hour() {
    let t = token(&[3; 32]);
    let mut all = Enrolments::default();
    for n in 0..LOCK_AFTER {
        let at = NOW + u64::try_from(n).expect("few");
        let s = stored(&t, at + 600);
        assert!(kept!(all.redeem(Some(&s), "wrong", at)).is_err());
    }
    assert!(all.locked(NOW + 10));
    let s = stored(&t, NOW + 700);
    assert!(kept!(all.redeem(Some(&s), &t, NOW + 10)).is_err(), "locked");
    let later = NOW + 3600 + 10;
    let s = stored(&t, later + 600);
    assert_eq!(kept!(all.redeem(Some(&s), &t, later)), Ok(()));
}

#[test]
fn enrolments_stored_empty_still_read() {
    let all: Enrolments = serde_json::from_str("{}").expect("reads");
    assert_eq!(all, Enrolments::default());
}
