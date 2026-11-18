//! `service.bind` and `service.unbind`'s offers: binding an Access service token to a service
//! account, by its client ID's hash, waits for the Administrator's passkey on
//! `/service-bind/<id>`; see
//! `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`, its step 9
//! amendments.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The operation that asks the Administrator to bind a credential.
pub const BIND: &str = "service.bind";
/// The operation that asks the Administrator to revoke one.
pub const UNBIND: &str = "service.unbind";
/// The page's path prefix.
pub const PAGE: &str = "/service-bind/";

/// A binding or revocation waiting for the Administrator's passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// The service account, or `agent`; empty for a revocation.
    #[serde(default)]
    pub account: String,
    /// The client ID's SHA-256, hex.
    pub client_hash: String,
    /// Whether it revokes rather than binds.
    #[serde(default)]
    pub unbind: bool,
    /// When the page's link expires, in Unix seconds.
    pub expires: u64,
}

impl Offer {
    /// The page's path, with `token`.
    #[must_use]
    pub fn path(&self, token: &str) -> String {
        format!("{PAGE}{}?grant={token}", self.id)
    }

    /// What the page asks the Administrator.
    #[must_use]
    pub fn title(&self) -> String {
        let short = self.client_hash.get(..12).unwrap_or(&self.client_hash);
        if self.unbind {
            format!("Revoke the service token {short}…")
        } else {
            format!("Bind the service token {short}… to `{}`", self.account)
        }
    }
}

/// Whether `hash` is a client ID's SHA-256: 64 lowercase hex digits.
#[must_use]
pub fn is_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The offers, kept by the `Instance`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offers {
    /// The last offer's number.
    #[serde(default)]
    pub seq: u32,
    /// Offers by their token's key.
    #[serde(default)]
    pub offers: BTreeMap<String, Offer>,
}

impl Offers {
    /// Offers to bind `client_hash` to `account`, or to revoke it when `account` is empty, under
    /// `token` until `expires`, dropping expired offers.
    ///
    /// # Errors
    ///
    /// When `client_hash` is not a SHA-256.
    pub fn offer(
        &mut self,
        account: &str,
        client_hash: &str,
        token: &str,
        now: u64,
        expires: u64,
    ) -> Result<Offer, String> {
        if !is_hash(client_hash) {
            return Err("`client_hash` is a client ID's SHA-256, 64 lowercase hex digits".into());
        }
        self.offers.retain(|_, o| o.expires > now);
        self.seq += 1;
        let offer = Offer {
            id: self.seq,
            account: account.to_string(),
            client_hash: client_hash.to_string(),
            unbind: account.is_empty(),
            expires,
        };
        self.offers
            .insert(crate::approval::grants::key(token), offer.clone());
        Ok(offer)
    }

    /// The offer `token` names, without using it.
    ///
    /// # Errors
    ///
    /// When the token names none, or it has expired.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Offer, String> {
        let o = self
            .offers
            .get(&crate::approval::grants::key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        if o.expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        Ok(o)
    }

    /// Spends the offer `token` names.
    ///
    /// # Errors
    ///
    /// When the token names no live offer.
    pub fn take(&mut self, token: &str, now: u64) -> Result<Offer, String> {
        let o = self.peek(token, now)?.clone();
        self.offers.remove(&crate::approval::grants::key(token));
        Ok(o)
    }
}

/// The offer number in a page path `/service-bind/<id>`.
#[must_use]
pub fn parse(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PAGE)?;
    let id = rest.split_once('?').map_or(rest, |(p, _)| p);
    (!id.starts_with('0')).then(|| id.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_000_000;

    fn hash() -> String {
        super::super::token_id("client-1")
    }

    #[test]
    fn an_offer_names_only_the_hash_and_is_taken_once() {
        let mut all = Offers::default();
        let o = all.offer("jobs", &hash(), "tok", NOW, NOW + 600).unwrap();
        assert_eq!(o.path("tok"), "/service-bind/1?grant=tok");
        assert!(!o.unbind);
        assert!(!serde_json::to_string(&all).unwrap().contains("client-1"));
        assert_eq!(all.take("tok", NOW).map(|o| o.account), Ok("jobs".into()));
        assert!(all.take("tok", NOW).is_err());
    }

    #[test]
    fn an_offer_refuses_a_value_that_is_not_a_hash() {
        let mut all = Offers::default();
        assert!(
            all.offer("jobs", "client-1", "tok", NOW, NOW + 600)
                .is_err()
        );
        assert!(
            all.offer("", &hash().to_uppercase(), "tok", NOW, NOW + 600)
                .is_err()
        );
        assert!(
            all.offer("", &hash(), "tok", NOW, NOW + 600)
                .unwrap()
                .unbind
        );
    }

    #[test]
    fn a_page_path_names_its_offer() {
        assert_eq!(parse("/service-bind/7?grant=x"), Some(7));
        assert_eq!(parse("/service-bind/07"), None);
        assert_eq!(parse("/service-bind/"), None);
        assert_eq!(parse("/pool-resume/7"), None);
    }
}
