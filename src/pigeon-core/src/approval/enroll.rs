//! One-time enrolment: the break-glass that registers a passkey when none is left; see
//! `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The furthest ahead an enrolment's expiry may be, in seconds.
pub const MAX_AHEAD: u64 = 60 * 60;

/// How many wrong tokens lock enrolment, within [`LOCK_WINDOW`] seconds.
pub const LOCK_AFTER: usize = 5;
/// The window those wrong tokens are counted in.
pub const LOCK_WINDOW: u64 = 60 * 60;

/// How many used enrolments are remembered.
const KEPT: usize = 16;

/// The token the owner types, from `random` bytes: lowercase hex.
#[must_use]
pub fn token(random: &[u8]) -> String {
    super::grants::hex(random)
}

/// What the `PASSKEY_ENROLL` secret holds for `token`, good until `expires`:
/// `<expires>.<SHA-256 of the token, hex>`. The token itself is never stored.
#[must_use]
pub fn stored(token: &str, expires: u64) -> String {
    format!("{expires}.{}", digest(token))
}

fn digest(token: &str) -> String {
    super::grants::hex(&Sha256::digest(token.as_bytes()))
}

/// The enrolments used so far and the wrong tokens tried, as the `Instance` keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrolments {
    /// The digests of the tokens already used.
    #[serde(default)]
    pub used: Vec<String>,
    /// When wrong tokens were tried, within [`LOCK_WINDOW`].
    #[serde(default)]
    pub failures: Vec<u64>,
}

impl Enrolments {
    /// Whether [`LOCK_AFTER`] wrong tokens in the last [`LOCK_WINDOW`] lock enrolment at `now`.
    #[must_use]
    pub fn locked(&self, now: u64) -> bool {
        self.failures
            .iter()
            .filter(|at| **at + LOCK_WINDOW > now)
            .count()
            >= LOCK_AFTER
    }

    /// Uses up `token` against `secret`, the `PASSKEY_ENROLL` value: the next state, and why it
    /// was refused. Only a wrong token against a live secret counts toward the lock.
    pub fn redeem(
        &self,
        secret: Option<&str>,
        token: &str,
        now: u64,
    ) -> (Self, Result<(), String>) {
        let mut next = self.clone();
        next.failures.retain(|at| *at + LOCK_WINDOW > now);
        let refuse = |next: Self, why: &str| (next, Err(why.to_string()));
        if next.locked(now) {
            return refuse(
                next,
                "five wrong tokens in the last hour; enrolment is locked for up to an hour",
            );
        }
        let parsed = secret
            .map(str::trim)
            .and_then(|s| s.split_once('.'))
            .and_then(|(expires, hash)| Some((expires.parse::<u64>().ok()?, hash)));
        let Some((expires, hash)) = parsed else {
            return refuse(next, "no enrolment is open");
        };
        if expires <= now {
            return refuse(next, "this enrolment has expired");
        }
        if expires > now + MAX_AHEAD {
            return refuse(next, "this enrolment's expiry is more than an hour ahead");
        }
        if next.used.iter().any(|u| u == hash) {
            return refuse(next, "this enrolment was already used");
        }
        if digest(token.trim()) != hash {
            next.failures.push(now);
            return refuse(next, "the token is wrong");
        }
        next.used.push(hash.to_string());
        let extra = next.used.len().saturating_sub(KEPT);
        next.used.drain(..extra);
        (next, Ok(()))
    }
}
