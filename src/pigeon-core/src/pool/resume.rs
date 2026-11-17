//! `pool.resume`'s offers: a pool paused by a tripwire or `pool.pause` resumes only on the
//! owner's passkey, on `/pool-resume/<pool>/<id>`; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`, its 2026-11-17
//! amendment "Step 3b3 lands in two changes before step 9".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The operation that asks the owner to resume a pool.
pub const OPERATION: &str = "pool.resume";
/// The page's path prefix.
pub const PAGE: &str = "/pool-resume/";

/// A resume waiting for the owner's passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// The pool it resumes.
    pub pool: String,
    /// When the page's link expires, in Unix seconds.
    pub expires: u64,
}

impl Offer {
    /// The page's path, with `token`.
    #[must_use]
    pub fn path(&self, token: &str) -> String {
        format!("{PAGE}{}/{}?grant={token}", self.pool, self.id)
    }
}

/// A pool's offers, kept by its `Pool` object.
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
    /// Offers to resume `pool` under `token` until `expires`, dropping expired offers.
    pub fn offer(&mut self, pool: &str, token: &str, now: u64, expires: u64) -> Offer {
        self.offers.retain(|_, o| o.expires > now);
        self.seq += 1;
        let offer = Offer {
            id: self.seq,
            pool: pool.to_string(),
            expires,
        };
        self.offers
            .insert(crate::approval::grants::key(token), offer.clone());
        offer
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

/// The pool and offer number in a page path `/pool-resume/<pool>/<id>`.
#[must_use]
pub fn parse(path: &str) -> Option<(&str, u32)> {
    let rest = path.strip_prefix(PAGE)?;
    let rest = rest.split_once('?').map_or(rest, |(p, _)| p);
    let (pool, id) = rest.split_once('/')?;
    let named = !pool.is_empty()
        && pool
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let id = (!id.starts_with('0')).then(|| id.parse().ok()).flatten()?;
    named.then_some((pool, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_000_000;

    #[test]
    fn an_offer_is_peeked_then_taken_once() {
        let mut all = Offers::default();
        let o = all.offer("anthropic", "tok", NOW, NOW + 600);
        assert_eq!(o.id, 1);
        assert_eq!(o.path("tok"), "/pool-resume/anthropic/1?grant=tok");
        assert_eq!(all.peek("tok", NOW).map(|o| o.id), Ok(1));
        assert_eq!(all.take("tok", NOW).map(|o| o.pool), Ok("anthropic".into()));
        assert!(all.take("tok", NOW).is_err());
    }

    #[test]
    fn an_expired_or_unknown_offer_is_refused() {
        let mut all = Offers::default();
        all.offer("anthropic", "tok", NOW, NOW + 600);
        assert!(all.peek("tok", NOW + 600).is_err());
        assert!(all.peek("other", NOW).is_err());
        all.offer("anthropic", "next", NOW + 700, NOW + 1300);
        assert_eq!(all.offers.len(), 1);
    }

    #[test]
    fn a_page_path_names_its_pool_and_offer() {
        assert_eq!(parse("/pool-resume/anthropic/3"), Some(("anthropic", 3)));
        assert_eq!(
            parse("/pool-resume/anthropic/3?grant=t"),
            Some(("anthropic", 3))
        );
        assert_eq!(parse("/pool-resume/anthropic/03"), None);
        assert_eq!(parse("/pool-resume/Anthropic/3"), None);
        assert_eq!(parse("/pool-resume//3"), None);
        assert_eq!(parse("/ci-window/3"), None);
    }
}
