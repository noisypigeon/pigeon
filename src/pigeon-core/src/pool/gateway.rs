//! What differs between model gateways, as a pure trait; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`.

use serde::{Deserialize, Serialize};

use super::anthropic;
use super::{Micros, config::Pool, ledger::Signal};

/// The usage a gateway reports for one response, in tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// The model that answered.
    pub model: String,
    /// Uncached input tokens.
    pub input: u64,
    /// Cache writes, 5 minute.
    pub write_5m: u64,
    /// Cache writes, 1 hour.
    pub write_1h: u64,
    /// Cache writes the response did not split by lifetime.
    pub write_unsplit: u64,
    /// Cache reads.
    pub read: u64,
    /// Output tokens.
    pub output: u64,
    /// Whether the response was billed as US-only inference.
    pub us_only: bool,
    /// Server tool uses.
    pub server_tools: u64,
    /// Whether the response ended, so the usage is final.
    pub complete: bool,
}

/// A request a gateway admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The gateway path.
    pub path: String,
    /// The model asked for.
    pub model: String,
    /// The request body's size in bytes.
    pub body_bytes: u64,
    /// The output tokens it may use.
    pub max_tokens: u64,
    /// Whether the response streams.
    pub stream: bool,
}

/// Why a gateway refused a request; the text starts `pool: `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

/// What differs between model gateways; every method is pure.
pub trait Gateway: Sync {
    /// The name `.config/pool.yaml` gives it.
    fn name(&self) -> &'static str;
    /// Admit or refuse a request.
    ///
    /// # Errors
    /// A [`Refused`] naming the first check the request fails.
    fn admit(&self, path: &str, body: &[u8], models: &[String]) -> Result<Admitted, Refused>;
    /// The most an admitted request can cost.
    fn reserve(&self, a: &Admitted, pool: &Pool) -> Micros;
    /// The usage in a whole response body.
    fn usage_json(&self, body: &[u8]) -> Option<Usage>;
    /// `acc` updated by one streamed event's data.
    fn usage_event(&self, acc: &Usage, data: &str) -> Usage;
    /// What `u` costs at list price.
    fn cost(&self, u: &Usage, pool: &Pool) -> Micros;
    /// The tokens of `u` that count against a task's budget.
    fn fresh_tokens(&self, u: &Usage) -> u64;
    /// What an error response says about the pool.
    fn classify(&self, status: u16, body: &str) -> Signal;
    /// The header carrying the key.
    fn key_header(&self) -> &'static str;
    /// The request headers passed through.
    fn forwarded_headers(&self) -> &'static [&'static str];
    /// The response header naming the account, if any.
    fn account_header(&self) -> Option<&'static str>;
}

static MESSAGES_GATEWAY: anthropic::Messages = anthropic::Messages;

/// The gateway `.config/pool.yaml` names `name`.
#[must_use]
pub fn gateway(name: &str) -> Option<&'static dyn Gateway> {
    match name {
        "anthropic-messages" => Some(&MESSAGES_GATEWAY),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_messages_is_known() {
        let g = gateway("anthropic-messages").expect("known");
        assert_eq!(g.name(), "anthropic-messages");
    }

    #[test]
    fn other_names_are_not() {
        assert!(gateway("other").is_none());
        assert!(gateway("").is_none());
    }
}
