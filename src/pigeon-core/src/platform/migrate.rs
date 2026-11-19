//! `secrets.migrate`'s offers, and how each copied name is shown: the shared Worker secrets move
//! into the Secrets Store by an in-Worker copy, once the owner approves with a passkey; see
//! `.claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md`, its `Shared secrets
//! move to the Secrets Store by an in-Worker copy` amendment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::secrets::SHARED;

/// The operation.
pub const OPERATION: &str = "secrets.migrate";
/// The page's path prefix.
pub const PAGE: &str = "/secrets-migrate/";
/// Where the `Instance` keeps the offers.
pub const STORE: &str = "secrets-migrate-offers";
/// How long the minted write token lives, in seconds.
pub const TOKEN_TTL: u64 = 10 * 60;
/// The Cloudflare permission group the minted token holds, and nothing else.
pub const PERMISSION: &str = "Secrets Store Write";

/// `names`, checked: at least one, each in [`SHARED`], sorted and without repeats.
///
/// # Errors
///
/// Naming the first name outside [`SHARED`], or when there is none.
pub fn names(names: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let n = n.trim();
        if !SHARED.contains(&n) {
            return Err(format!("`{n}` is not a shared secret"));
        }
        if !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
    }
    if out.is_empty() {
        return Err("`names` lists at least one shared secret".into());
    }
    out.sort();
    Ok(out)
}

/// A secret's name in the Secrets Store: lower-kebab-case, as `journal-seal` is; the agent token,
/// kept under its stored Worker name, is `agent-token`.
#[must_use]
pub fn store_name(name: &str) -> String {
    if name == "BACKPLANE_TOKEN" {
        return "agent-token".into();
    }
    name.to_ascii_lowercase().replace('_', "-")
}

/// A value's fingerprint: the first 12 hex digits of HMAC-SHA-256 of `value` under `key`.
#[must_use]
pub fn fingerprint(key: &str, value: &str) -> String {
    crate::wire::link::derive(key, value)[..12].to_string()
}

/// What happened to one name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copied {
    /// The Worker secret's name.
    pub name: String,
    /// Its name in the store.
    pub store: String,
    /// `written`, `exists`, `missing`, or `failed: <step>`.
    pub outcome: String,
    /// The value's [`fingerprint`], when the Worker holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// The page's outcome: one line per name, never a value.
#[must_use]
pub fn summary(copied: &[Copied]) -> String {
    copied
        .iter()
        .map(|c| match &c.fingerprint {
            Some(f) => format!("{} → {}: {} ({f})", c.name, c.store, c.outcome),
            None => format!("{} → {}: {}", c.name, c.store, c.outcome),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A copy waiting for the owner's passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// The names it copies.
    pub names: Vec<String>,
    /// When the page's link expires, in Unix seconds.
    pub expires: u64,
}

impl Offer {
    /// The page's path, with `token`.
    #[must_use]
    pub fn path(&self, token: &str) -> String {
        format!("{PAGE}{}?grant={token}", self.id)
    }

    /// What the page asks the owner.
    #[must_use]
    pub fn title(&self) -> String {
        match self.names.as_slice() {
            [one] => format!("Copy {one} into the Secrets Store"),
            many => format!("Copy {} shared secrets into the Secrets Store", many.len()),
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
    /// The offers with one more, copying `names` under `token` until `expires`, less the expired
    /// ones; and that offer.
    ///
    /// # Errors
    ///
    /// When [`names`] refuses them.
    pub fn offer(
        self,
        names: &[String],
        token: &str,
        now: u64,
        expires: u64,
    ) -> Result<(Self, Offer), String> {
        let names = self::names(names)?;
        let seq = self.seq + 1;
        let offer = Offer {
            id: seq,
            names,
            expires,
        };
        let mut offers: BTreeMap<String, Offer> = self
            .offers
            .into_iter()
            .filter(|(_, o)| o.expires > now)
            .collect();
        offers.insert(crate::approval::grants::key(token), offer.clone());
        Ok((Self { seq, offers }, offer))
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

    /// The offers without the one `token` names, and that offer.
    ///
    /// # Errors
    ///
    /// When the token names no live offer.
    pub fn take(self, token: &str, now: u64) -> Result<(Self, Offer), String> {
        let o = self.peek(token, now)?.clone();
        let key = crate::approval::grants::key(token);
        let offers = self.offers.into_iter().filter(|(k, _)| *k != key).collect();
        Ok((
            Self {
                seq: self.seq,
                offers,
            },
            o,
        ))
    }
}

/// The offer number in a page path `/secrets-migrate/<id>`.
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

    fn list(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn only_shared_names_are_copied_once_each() {
        assert_eq!(
            names(&list(&["SESSION_KEY", "APPROVER_TOTP", "SESSION_KEY"])),
            Ok(list(&["APPROVER_TOTP", "SESSION_KEY"]))
        );
        assert!(names(&list(&["DEPLOY_OUTPUTS"])).is_err());
        assert!(names(&list(&["journal-seal"])).is_err());
        assert!(names(&[]).is_err());
    }

    #[test]
    fn store_names_are_lower_kebab() {
        assert_eq!(store_name("PLATFORM_STATE_KEY"), "platform-state-key");
        assert_eq!(store_name("BACKPLANE_TOKEN"), "agent-token");
    }

    #[test]
    fn a_fingerprint_is_twelve_hex_digits_that_name_no_value() {
        let f = fingerprint("key", "value");
        assert_eq!(f.len(), 12);
        assert!(f.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(f, fingerprint("key", "other"));
        assert_ne!(f, fingerprint("other", "value"));
    }

    #[test]
    fn an_offer_is_spent_once_and_expires() {
        let (o, offer) = Offers::default()
            .offer(&list(&["SESSION_KEY"]), "t", NOW, NOW + 60)
            .unwrap();
        assert_eq!(offer.path("t"), "/secrets-migrate/1?grant=t");
        assert_eq!(parse("/secrets-migrate/1?grant=t"), Some(1));
        assert!(o.peek("t", NOW + 61).is_err());
        let (o, taken) = o.clone().take("t", NOW).unwrap();
        assert_eq!(taken, offer);
        assert!(o.clone().take("t", NOW).is_err());
        assert!(o.offer(&list(&["NOPE"]), "u", NOW, NOW + 60).is_err());
    }

    #[test]
    fn the_summary_names_outcomes_and_fingerprints_only() {
        let s = summary(&[
            Copied {
                name: "SESSION_KEY".into(),
                store: "session-key".into(),
                outcome: "written".into(),
                fingerprint: Some("0123456789ab".into()),
            },
            Copied {
                name: "GITHUB_TOKEN".into(),
                store: "github-token".into(),
                outcome: "missing".into(),
                fingerprint: None,
            },
        ]);
        assert_eq!(
            s,
            "SESSION_KEY → session-key: written (0123456789ab)\nGITHUB_TOKEN → github-token: missing"
        );
    }
}
