//! R2's access key pair and the hashes a sync compares and signs with; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §7.

pub use common::hex::{hex, md5_hex, sha256_hex};

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
