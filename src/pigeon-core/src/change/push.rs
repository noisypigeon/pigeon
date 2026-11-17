//! A push's refusal, read from git's stderr: transient, or settled; see
//! `.claude/decisions/2026-11-16-development-merge-path-measured-faster.md`.

use super::types::STALE_BASE;

/// Seconds the merge job waits before each retry of a push refused for now.
pub const BACKOFF: [u64; 4] = [2, 4, 8, 16];

/// What git's stderr says, lowercased, when the remote refused the push for now.
const TRANSIENT: [&str; 13] = [
    "receive_pack_busy",
    "http 429",
    "http 502",
    "http 503",
    "http 504",
    "error: 429",
    "error: 502",
    "error: 503",
    "error: 504",
    "rpc failed",
    "unexpected disconnect",
    "connection reset",
    "early eof",
];

/// What git's stderr says, lowercased, when the push was refused for good.
const SETTLED: [&str; 3] = ["[rejected]", "fetch first", "non-fast-forward"];

/// Whether a push's `stderr` is a transient refusal, worth trying again.
#[must_use]
pub fn transient(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    if stderr.contains(STALE_BASE) || SETTLED.iter().any(|p| s.contains(p)) {
        return false;
    }
    TRANSIENT.iter().any(|p| s.contains(p))
}
