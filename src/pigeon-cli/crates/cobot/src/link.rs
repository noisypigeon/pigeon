//! The signed Worker–Container link: request and response signatures under `BACKPLANE_TOKEN`,
//! per the "Phase 3 spike report" amendment §2 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`. Pure: the caller
//! supplies the clock and the randomness.

use std::fmt::Write as _;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// A signed request's Unix time in seconds.
pub const TIMESTAMP_HEADER: &str = "x-pigeon-timestamp";

/// A signed request's nonce: 32 to 64 hex digits, never reused.
pub const NONCE_HEADER: &str = "x-pigeon-nonce";

/// A request's or response's HMAC-SHA-256, in hex.
pub const SIGNATURE_HEADER: &str = "x-pigeon-signature";

/// How far a signed request's timestamp may be from the server's clock, in seconds.
pub const SKEW: u64 = 60;

/// The headers a signed request carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    /// [`TIMESTAMP_HEADER`]'s value.
    pub timestamp: u64,
    /// [`NONCE_HEADER`]'s value.
    pub nonce: String,
    /// [`SIGNATURE_HEADER`]'s value.
    pub signature: String,
}

/// A response whose signature is missing or does not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unverified;

impl std::fmt::Display for Unverified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the response's signature is missing or wrong")
    }
}

impl std::error::Error for Unverified {}

/// Signs a request at Unix time `now`, with a nonce from 32 random bytes.
#[must_use]
pub fn sign(
    token: &str,
    method: &str,
    path: &str,
    now: u64,
    nonce: [u8; 32],
    body: &[u8],
) -> Signed {
    let nonce = hex(&nonce);
    Signed {
        signature: sign_request(token, method, path, now, &nonce, body),
        timestamp: now,
        nonce,
    }
}

/// Checks a response to the request that carried `nonce`.
///
/// # Errors
///
/// [`Unverified`] if `signature` is missing or is not the one for `status` and `body`.
pub fn verify_response(
    token: &str,
    nonce: &str,
    status: u16,
    body: &[u8],
    signature: Option<&str>,
) -> Result<(), Unverified> {
    let want = sign_response(token, nonce, status, body);
    match signature {
        Some(s) if constant_eq(s.as_bytes(), want.as_bytes()) => Ok(()),
        _ => Err(Unverified),
    }
}

/// The signature a client sends for a request: HMAC-SHA-256 under `token` over the method, path,
/// timestamp, nonce and the body's SHA-256, in hex.
#[must_use]
pub fn sign_request(
    token: &str,
    method: &str,
    path: &str,
    timestamp: u64,
    nonce: &str,
    body: &[u8],
) -> String {
    hmac_hex(
        token,
        &format!(
            "pigeon-v0-request\n{method}\n{path}\n{timestamp}\n{}\n{}",
            nonce.to_ascii_lowercase(),
            hex(&Sha256::digest(body))
        ),
    )
}

/// The signature `serve` sends with the response to a signed request: HMAC-SHA-256 under `token`
/// over the request's nonce, the status and the body's SHA-256, in hex.
#[must_use]
pub fn sign_response(token: &str, nonce: &str, status: u16, body: &[u8]) -> String {
    hmac_hex(
        token,
        &format!(
            "pigeon-v0-response\n{}\n{status}\n{}",
            nonce.to_ascii_lowercase(),
            hex(&Sha256::digest(body))
        ),
    )
}

/// Compares in time independent of where the inputs first differ.
#[must_use]
pub fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hmac_hex(token: &str, message: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(message.as_bytes());
    hex(&mac.finalize().into_bytes())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::{Unverified, sign, sign_request, sign_response, verify_response};

    const TOKEN: &str = "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk";
    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    const BODY: &[u8] = br#"{"operation":"plan"}"#;

    #[test]
    fn signatures_match_known_answers() {
        assert_eq!(
            sign_request(TOKEN, "POST", "/v0/run", 1_700_000_000, NONCE, BODY),
            "e51ccb5e45584ee06a43bf80340101b7c3c745755f4db4384293eaab1af9d980"
        );
        assert_eq!(
            sign_response(TOKEN, NONCE, 200, BODY),
            "7f24fe61415e3df68145ddb53335be39a073e19d20d64474838eb18e5f79cc53"
        );
        assert_eq!(
            sign_response(TOKEN, &NONCE.to_ascii_uppercase(), 200, BODY),
            sign_response(TOKEN, NONCE, 200, BODY),
            "the nonce's case does not matter"
        );
    }

    #[test]
    fn sign_uses_the_clock_and_nonce_given() {
        let signed = sign(TOKEN, "GET", "/v0/operations", 7, [0xab; 32], b"");
        assert_eq!(signed.timestamp, 7);
        assert_eq!(signed.nonce, "ab".repeat(32));
        assert_eq!(
            signed.signature,
            sign_request(TOKEN, "GET", "/v0/operations", 7, &signed.nonce, b"")
        );
    }

    #[test]
    fn a_response_verifies_only_as_signed() {
        let good = sign_response(TOKEN, NONCE, 200, BODY);
        assert_eq!(
            verify_response(TOKEN, NONCE, 200, BODY, Some(&good)),
            Ok(())
        );
        for (nonce, status, body, sig) in [
            (NONCE, 200, BODY, None),
            (NONCE, 409, BODY, Some(good.as_str())),
            (NONCE, 200, &b"{}"[..], Some(good.as_str())),
            (
                "ffffffffffffffffffffffffffffffff",
                200,
                BODY,
                Some(good.as_str()),
            ),
        ] {
            assert_eq!(
                verify_response(TOKEN, nonce, status, body, sig),
                Err(Unverified)
            );
        }
        let other = sign_response("another token, also 32 bytes long", NONCE, 200, BODY);
        assert_eq!(
            verify_response(TOKEN, NONCE, 200, BODY, Some(&other)),
            Err(Unverified)
        );
    }
}
