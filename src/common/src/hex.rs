//! Lowercase hex, and the digests R2 and signing compare in it.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

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
