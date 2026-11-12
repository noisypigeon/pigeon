//! The owner's authenticator code: RFC 6238, each step once, a lockout after wrong codes.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

use backplane::totp::{self, Guard, LOCKOUT, MAX_FAILURES, STEP};

const SECRET: &[u8] = b"12345678901234567890";

fn six(n: u32) -> String {
    format!("{n:06}")
}

#[test]
fn codes_match_rfc_6238_sha1() {
    for (t, want) in [
        (59, 287_082),
        (1_111_111_109, 81_804),
        (1_111_111_111, 50_471),
        (1_234_567_890, 5_924),
        (2_000_000_000, 279_037),
        (20_000_000_000, 353_130),
    ] {
        assert_eq!(totp::code(SECRET, t / STEP), want, "t={t}");
    }
}

#[test]
fn base32_round_trips_and_forgives_case_spaces_and_padding() {
    let s = totp::base32_encode(SECRET);
    assert_eq!(s, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    assert_eq!(totp::base32_decode(&s).expect("decode"), SECRET);
    assert_eq!(
        totp::base32_decode("gezd gnbv-gy3t qojq gezdgnbvgy3tqojq==").expect("decode"),
        SECRET
    );
    assert!(totp::base32_decode("not base32!").is_err());
    assert!(totp::base32_decode("").is_err());
}

#[test]
fn a_code_is_accepted_in_its_window_once() {
    let now = 1_111_111_111;
    let mut g = Guard::default();
    let code = six(totp::code(SECRET, now / STEP - 1));
    assert_eq!(
        kept!(g.verify(SECRET, &code, now)),
        Ok(now / STEP - 1),
        "one step back"
    );
    let again = kept!(g.verify(SECRET, &code, now)).expect_err("replay");
    assert!(again.contains("already used"), "{again}");
    let current = six(totp::code(SECRET, now / STEP));
    assert!(
        kept!(g.verify(SECRET, &current, now)).is_ok(),
        "a later step"
    );
    let mut fresh = Guard::default();
    let stale = six(totp::code(SECRET, now / STEP - 2));
    assert!(
        kept!(fresh.verify(SECRET, &stale, now)).is_err(),
        "outside the window"
    );
    assert!(kept!(fresh.verify(SECRET, " 12a456", now)).is_err());
    assert!(kept!(fresh.verify(SECRET, "1234567", now)).is_err());
}

#[test]
fn wrong_codes_lock_approvals_for_an_hour() {
    let now = 2_000_000_000;
    let right = six(totp::code(SECRET, now / STEP));
    let wrong = six((totp::code(SECRET, now / STEP) + 1) % 1_000_000);
    let mut g = Guard::default();
    for i in 0..MAX_FAILURES {
        assert!(kept!(g.verify(SECRET, &wrong, now + i as u64)).is_err());
    }
    assert!(g.locked(now + 10));
    let why = kept!(g.verify(SECRET, &right, now + 10)).expect_err("locked");
    assert!(why.contains("locked"), "{why}");
    let later = now + LOCKOUT + MAX_FAILURES as u64;
    assert!(!g.locked(later));
    let code = six(totp::code(SECRET, later / STEP));
    assert!(
        kept!(g.verify(SECRET, &code, later)).is_ok(),
        "the lockout ends"
    );
}
