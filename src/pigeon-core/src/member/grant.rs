//! `member.grant` and `member.revoke`'s offers: granting a Standard member a capability, or
//! revoking it, waits for the Administrator's passkey on `/member-grant/<id>`; see
//! `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`, its "rest in
//! parallel" amendment (A1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The operation that asks the Administrator to grant a capability.
pub const GRANT: &str = "member.grant";
/// The operation that asks the Administrator to revoke one.
pub const REVOKE: &str = "member.revoke";
/// The page's path prefix.
pub const PAGE: &str = "/member-grant/";

/// A grant or revocation waiting for the Administrator's passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// The member's account.
    pub account: String,
    /// The capability, one of [`super::CAPABILITIES`].
    pub capability: String,
    /// What the grant is limited to, when it takes a scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// When the grant lapses; never when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    /// Whether it revokes rather than grants.
    #[serde(default)]
    pub revoke: bool,
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
        let what = match &self.scope {
            Some(s) => format!("`{}` for `{s}`", self.capability),
            None => format!("`{}`", self.capability),
        };
        if self.revoke {
            format!("Revoke {what} from `{}`", self.account)
        } else {
            format!("Grant {what} to `{}`", self.account)
        }
    }
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
    /// Offers `draft` (its `id` and `expires` set here) under `token` until `expires`, dropping
    /// expired offers.
    ///
    /// # Errors
    ///
    /// When the account is empty, the capability is unknown, or a grant names one that can't
    /// be granted yet; [`super::Members::grant`] checks the rest when the passkey redeems it.
    pub fn offer(
        &mut self,
        mut draft: Offer,
        token: &str,
        now: u64,
        expires: u64,
    ) -> Result<Offer, String> {
        if draft.account.trim().is_empty() {
            return Err("`account` names the member".into());
        }
        if !super::CAPABILITIES.contains(&draft.capability.as_str()) {
            return Err(format!("`{}` is not a capability", draft.capability));
        }
        if !draft.revoke && !super::GRANTABLE.contains(&draft.capability.as_str()) {
            return Err(format!("`{}` can't be granted yet", draft.capability));
        }
        if draft.until.is_some_and(|t| t <= now) {
            return Err("a grant must expire in the future".into());
        }
        self.offers.retain(|_, o| o.expires > now);
        self.seq += 1;
        draft.id = self.seq;
        draft.expires = expires;
        self.offers
            .insert(crate::approval::grants::key(token), draft.clone());
        Ok(draft)
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

/// The offer number in a page path `/member-grant/<id>`.
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

    fn draft(capability: &str, revoke: bool) -> Offer {
        Offer {
            id: 0,
            account: "m1".into(),
            capability: capability.into(),
            scope: None,
            until: None,
            revoke,
            expires: 0,
        }
    }

    #[test]
    fn an_offer_is_numbered_and_taken_once() {
        let mut all = Offers::default();
        let o = all
            .offer(draft("icbc", false), "tok", NOW, NOW + 600)
            .unwrap();
        assert_eq!(o.path("tok"), "/member-grant/1?grant=tok");
        assert_eq!(o.title(), "Grant `icbc` to `m1`");
        assert_eq!(
            all.take("tok", NOW).map(|o| o.capability),
            Ok("icbc".into())
        );
        assert!(all.take("tok", NOW).is_err());
    }

    #[test]
    fn a_grant_offer_names_only_a_grantable_capability() {
        let mut all = Offers::default();
        assert!(
            all.offer(draft("approve", false), "a", NOW, NOW + 600)
                .is_err()
        );
        assert!(
            all.offer(draft("blog", false), "b", NOW, NOW + 600)
                .is_err()
        );
        assert!(all.offer(draft("blog", true), "c", NOW, NOW + 600).is_ok());
        let mut past = draft("icbc", false);
        past.until = Some(NOW);
        assert!(all.offer(past, "d", NOW, NOW + 600).is_err());
        let mut nobody = draft("icbc", false);
        nobody.account = " ".into();
        assert!(all.offer(nobody, "e", NOW, NOW + 600).is_err());
    }

    #[test]
    fn a_page_path_names_its_offer() {
        assert_eq!(parse("/member-grant/7?grant=x"), Some(7));
        assert_eq!(parse("/member-grant/07"), None);
        assert_eq!(parse("/member-grant/"), None);
        assert_eq!(parse("/service-bind/7"), None);
    }
}
