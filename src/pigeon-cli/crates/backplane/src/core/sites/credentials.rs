//! R2's access key pair and the hashes a sync compares and signs with; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §7.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// An access key pair; the secret never appears in `Debug` output.
#[derive(Clone)]
pub struct Credentials {
    /// The access key id.
    pub access_key: String,
    /// The secret access key.
    pub secret_key: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .finish_non_exhaustive()
    }
}

/// Lowercase hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Hex MD5 of `bytes`, as R2 reports a single-part object's `ETag`.
#[must_use]
pub fn md5_hex(bytes: &[u8]) -> String {
    hex(&md5::Md5::digest(bytes))
}
