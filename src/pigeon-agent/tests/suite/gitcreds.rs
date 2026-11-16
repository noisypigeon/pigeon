//! The git credentials path's decisions: retries and reuse; see
//! `.claude/decisions/2026-11-06-security-git-front-door-spike.md`.

use backplane::gitcreds::{BACKOFF_MS, Kept, retryable};

#[test]
fn only_a_missing_answer_too_many_requests_or_a_server_error_is_retried() {
    for s in [None, Some(429), Some(500), Some(502), Some(503), Some(599)] {
        assert!(retryable(s), "{s:?}");
    }
    for s in [
        Some(400),
        Some(401),
        Some(403),
        Some(404),
        Some(409),
        Some(600),
    ] {
        assert!(!retryable(s), "{s:?}");
    }
    assert_eq!(BACKOFF_MS, [250, 1000], "two retries, briefly");
}

#[test]
fn a_token_is_reused_while_half_its_lifetime_remains_and_only_for_its_scope() {
    let kept = Kept::default().with_token(false, "r".into(), 1000, 120);
    assert_eq!(kept.token(false, 120, 1000), Some("r"));
    assert_eq!(
        kept.token(false, 120, 1060),
        Some("r"),
        "60 of 120 seconds left"
    );
    assert_eq!(kept.token(false, 120, 1061), None, "less than half left");
    assert_eq!(
        kept.token(true, 120, 1000),
        None,
        "a read token never writes"
    );
    assert_eq!(
        kept.token(false, 900, 1000),
        None,
        "a caller wanting 15 minutes gets a token with at least half of that"
    );
    let both = kept.with_token(true, "w".into(), 1000, 120);
    assert_eq!(both.token(true, 120, 1010), Some("w"));
    assert_eq!(both.token(false, 120, 1010), Some("r"));
}

#[test]
fn the_account_and_remote_are_kept_beside_the_tokens() {
    let kept = Kept::default()
        .with_token(true, "w".into(), 0, 120)
        .with_repository("acct".into(), "https://git.example/r.git".into());
    assert_eq!(kept.account.as_deref(), Some("acct"));
    assert_eq!(kept.remote.as_deref(), Some("https://git.example/r.git"));
    assert_eq!(kept.token(true, 120, 0), Some("w"));
}
