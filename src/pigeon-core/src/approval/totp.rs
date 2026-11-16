//! The owner's authenticator code, which proves a card's approval is theirs: TOTP per RFC 6238
//! (HMAC-SHA1, 6 digits, 30-second steps), each step used once, and a lockout after repeated
//! failures; see the amendment of 2026-11-07 to
//! `.claude/decisions/2026-11-06-development-control-plane-approvals-images-state.md` §1.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;

/// Seconds per step.
pub const STEP: u64 = 30;

/// Steps either side of now that a code may come from.
pub const WINDOW: u64 = 1;

/// Wrong codes allowed in [`LOCKOUT`] seconds.
pub const MAX_FAILURES: usize = 5;

/// How long failures count, and a lockout lasts.
pub const LOCKOUT: u64 = 3600;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, unpadded.
#[must_use]
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((acc >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((acc << (5 - bits)) & 31) as usize]));
    }
    out
}

/// RFC 4648 base32, case-insensitive, ignoring padding, spaces and dashes.
///
/// # Errors
///
/// A message for a character outside the alphabet, or an empty secret.
pub fn base32_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes().filter(|c| !matches!(c, b'=' | b' ' | b'-')) {
        let c = c.to_ascii_uppercase();
        let v = ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or("the secret is not base32")?;
        acc = (acc << 5) | u32::try_from(v).unwrap_or_default();
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or_default());
        }
    }
    if out.is_empty() {
        return Err("the secret is empty".into());
    }
    Ok(out)
}

/// The code for `step`.
///
/// # Panics
///
/// Never: HMAC takes a key of any length.
#[must_use]
pub fn code(secret: &[u8], step: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = usize::from(h[19] & 0x0f);
    let bin = (u32::from(h[o]) & 0x7f) << 24
        | u32::from(h[o + 1]) << 16
        | u32::from(h[o + 2]) << 8
        | u32::from(h[o + 3]);
    bin % 1_000_000
}

/// What remembers past codes: the last step used, and recent failures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    /// The last step a code was accepted for.
    #[serde(default)]
    pub last_step: u64,
    /// When each recent wrong code came, in Unix seconds.
    #[serde(default)]
    pub failures: Vec<u64>,
}

impl Guard {
    /// Whether wrong codes have locked approvals at `now`.
    #[must_use]
    pub fn locked(&self, now: u64) -> bool {
        self.failures.iter().filter(|t| now < **t + LOCKOUT).count() >= MAX_FAILURES
    }

    /// Checks `typed` against `secret` at `now`: within the window, for a step after the last
    /// one used, and not while locked: the next guard, which records the step or the failure,
    /// and the step or why it was refused.
    pub fn verify(&self, secret: &[u8], typed: &str, now: u64) -> (Self, Result<u64, String>) {
        let mut next = self.clone();
        next.failures.retain(|t| now < *t + LOCKOUT);
        if next.locked(now) {
            let why = format!(
                "{MAX_FAILURES} wrong codes in the last hour; approvals by code are locked for up to an hour"
            );
            return (next, Err(why));
        }
        let typed = typed.trim();
        let found = typed.len() == 6 && typed.bytes().all(|b| b.is_ascii_digit());
        let typed: u32 = if found {
            typed.parse().unwrap_or(u32::MAX)
        } else {
            u32::MAX
        };
        let now_step = now / STEP;
        let hit = (now_step.saturating_sub(WINDOW)..=now_step + WINDOW)
            .find(|s| code(secret, *s) == typed);
        match hit {
            Some(step) if step > next.last_step => {
                next.last_step = step;
                (next, Ok(step))
            }
            Some(_) => (
                next,
                Err("that code was already used; wait for the next one".into()),
            ),
            None => {
                next.failures.push(now);
                (next, Err("the code is wrong".into()))
            }
        }
    }
}
