//! One-time grants: an approval offered to a person as a link, bound to an operation, a change's
//! head and an expiry; see the amendment of 2026-11-06 to
//! `.claude/decisions/2026-11-06-development-control-plane-approvals-images-state.md` §1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::changes::{Change, State};

/// How long a card's grant lives, in seconds.
pub const TTL: u64 = 15 * 60;

/// How long a page's grant lives, in seconds: long enough to approve from a phone later; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.
pub const PAGE_TTL: u64 = 24 * 60 * 60;

/// The operations a grant offers: those only a person runs on a change.
pub const OPERATIONS: [&str; 5] = [
    "change.authorize",
    "change.approve",
    "change.merge",
    "change.close",
    crate::control::BREAK_GLASS,
];

/// What a grant lets its redeemer do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The operation.
    pub operation: String,
    /// The change.
    pub number: u32,
    /// The change's head when it was minted; it must still be the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// When it expires, in Unix seconds.
    pub expires: u64,
    /// The tier it needs, from the head's tiers when minted; none is code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<crate::approvals::Tier>,
    /// When it was used; a used grant is kept until it expires, to say so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
}

impl Grant {
    /// Whether redeeming it needs the owner's code, not one tap.
    #[must_use]
    pub fn needs_code(&self) -> bool {
        self.tier != Some(crate::approvals::Tier::Confirm)
    }
}

/// Lowercase hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A token's key in storage: its SHA-256, so stored grants hold no live token.
#[must_use]
pub fn key(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Whether `operation` may be offered on `change` now.
///
/// # Errors
///
/// Why not.
pub fn offerable(change: &Change, operation: &str) -> Result<(), String> {
    let wanted = match operation {
        "change.authorize" => change.state == State::Proposed,
        "change.approve" => change.state == State::Checked && change.head.is_some(),
        "change.merge" => change.state == State::Approved,
        "change.close" => !matches!(change.state, State::Merged | State::Closed),
        crate::control::BREAK_GLASS => {
            !matches!(change.state, State::Merged | State::Closed)
                && crate::control::code_unpassed(change)
        }
        other => return Err(format!("`{other}` is not offered as a grant")),
    };
    if wanted {
        Ok(())
    } else {
        Err(format!(
            "{} is {}; `{operation}` doesn't apply",
            change.branch(),
            change.state.name()
        ))
    }
}

/// Whether `grant` still means something on `change`: its operation still applies and the head
/// is the one it was made for, so the inbox keeps it.
#[must_use]
pub fn still_applies(change: &Change, grant: &Grant) -> bool {
    grant.number == change.number
        && grant.head == change.head
        && offerable(change, &grant.operation).is_ok()
}

/// A change's open grants, by [`key`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grants(pub BTreeMap<String, Grant>);

impl Grants {
    /// Mints a grant of `operation` on `change` under `token`, dropping expired ones.
    ///
    /// # Errors
    ///
    /// When the operation doesn't apply to the change now.
    pub fn mint(
        &mut self,
        change: &Change,
        operation: &str,
        token: &str,
        now: u64,
    ) -> Result<Grant, String> {
        self.mint_for(change, operation, token, now, TTL)
    }

    /// As [`Self::mint`], living `ttl` seconds.
    ///
    /// # Errors
    ///
    /// When the operation doesn't apply to the change now.
    pub fn mint_for(
        &mut self,
        change: &Change,
        operation: &str,
        token: &str,
        now: u64,
        ttl: u64,
    ) -> Result<Grant, String> {
        offerable(change, operation)?;
        self.0.retain(|_, g| g.expires > now);
        let grant = Grant {
            operation: operation.to_string(),
            number: change.number,
            head: change.head.clone(),
            expires: now + ttl,
            tier: Some(crate::approvals::tier_of(change, operation)),
            used: None,
        };
        self.0.insert(key(token), grant.clone());
        Ok(grant)
    }

    /// The grant `token` names on change `number`, without using it.
    ///
    /// # Errors
    ///
    /// When there is none, or it has expired.
    pub fn peek(&self, token: &str, number: u32, now: u64) -> Result<&Grant, String> {
        let g = self
            .0
            .get(&key(token))
            .filter(|g| g.number == number)
            .ok_or("this link is not valid; ask for a new one")?;
        if g.used.is_some() {
            return Err(format!(
                "this link was already used: `{}` on change {} is done",
                g.operation, g.number
            ));
        }
        if g.expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        Ok(g)
    }

    /// Uses `token`'s grant on `change` for `operation`: once, before it expires, and only
    /// while the change's head is the one it was minted for. A grant found is spent even when
    /// refused, and kept as used until it expires.
    ///
    /// # Errors
    ///
    /// Why it can't be used.
    pub fn redeem(
        &mut self,
        token: &str,
        operation: &str,
        change: &Change,
        now: u64,
    ) -> Result<Grant, String> {
        self.peek(token, change.number, now)?;
        let stored = self
            .0
            .get_mut(&key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        stored.used = Some(now);
        let g = stored.clone();
        if g.operation != operation {
            return Err(format!(
                "this link is for `{}`, not `{operation}`",
                g.operation
            ));
        }
        if g.head != change.head {
            return Err(format!(
                "{} moved since this link was made; ask for a new one",
                change.branch()
            ));
        }
        Ok(g)
    }
}
