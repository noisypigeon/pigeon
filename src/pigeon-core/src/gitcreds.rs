//! What the git credentials path decides, apart from the Worker that fetches: which Cloudflare
//! API failures are retried and after how long, and which looked-up and minted values are kept
//! and reused; see `.claude/decisions/2026-11-06-security-git-front-door-spike.md`.

/// The pauses before each retry, in milliseconds: two retries after the first attempt.
pub const BACKOFF_MS: [u64; 2] = [250, 1000];

/// Whether a failed call is worth trying again: no answer at all, too many requests, or a
/// server error. Any other refusal (an invalid token, a missing permission) is not.
#[must_use]
pub fn retryable(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(s) => s == 429 || (500..600).contains(&s),
    }
}

/// A minted token and when it lapses, in Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Minted {
    /// The token.
    pub token: String,
    /// When it lapses.
    pub expires: u64,
}

/// What the credentials path keeps between requests: the account the API token sees, the
/// repository's remote, and the newest token of each scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    /// The account ID.
    pub account: Option<String>,
    /// The repository's remote URL.
    pub remote: Option<String>,
    /// The newest read token.
    pub read: Option<Minted>,
    /// The newest write token.
    pub write: Option<Minted>,
}

impl Kept {
    /// A kept token of `write`'s scope that still has at least half of `ttl` left at `now`.
    #[must_use]
    pub fn token(&self, write: bool, ttl: u64, now: u64) -> Option<&str> {
        let kept = if write { &self.write } else { &self.read };
        kept.as_ref()
            .filter(|m| m.expires >= now.saturating_add(ttl.div_ceil(2)))
            .map(|m| m.token.as_str())
    }

    /// This, with `token` of `write`'s scope, minted at `now` for `ttl` seconds.
    #[must_use]
    pub fn with_token(self, write: bool, token: String, now: u64, ttl: u64) -> Self {
        let minted = Some(Minted {
            token,
            expires: now.saturating_add(ttl),
        });
        if write {
            Self {
                write: minted,
                ..self
            }
        } else {
            Self {
                read: minted,
                ..self
            }
        }
    }

    /// This, with the account and remote looked up.
    #[must_use]
    pub fn with_repository(self, account: String, remote: String) -> Self {
        Self {
            account: Some(account),
            remote: Some(remote),
            ..self
        }
    }
}
