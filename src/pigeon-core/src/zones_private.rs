//! `zones.private`, the instance private zone store, and the offers of `zones.set`; see
//! `.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md` §1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The `Instance` key holding the private zones.
pub const STORE: &str = "zones.private";
/// The `Instance` key holding the pending offers; not journaled.
pub const OFFERS: &str = "zones.private.offers";
/// The operation that sets a private zone.
pub const SET: &str = "zones.set";
/// The operation that shows one private zone.
pub const SHOW: &str = "zones.show";
/// The operation that lists the private zones.
pub const LIST: &str = "zones.list";
/// The event a redeemed set appends.
pub const UPDATED: &str = "zones.updated";
/// The page prefix of an offer.
pub const PAGE: &str = "/zone-private/";

/// One private zone: what `--private` takes for an alias, and when it was set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The zone's name.
    pub zone: String,
    /// Its services.
    #[serde(default)]
    pub services: Map<String, Value>,
    /// Its records.
    #[serde(default)]
    pub records: Map<String, Value>,
    /// When it was set, in seconds.
    #[serde(default)]
    pub updated: u64,
}

/// The private zones by alias.
pub type Store = BTreeMap<String, Entry>;

/// An offer to set a private zone, redeemed with a passkey.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    /// The offer's number.
    pub id: u32,
    /// The alias to set.
    pub alias: String,
    /// The entry to store.
    pub entry: Entry,
    /// When the offer expires, in seconds.
    pub expires: u64,
}

impl Offer {
    /// The page path for this offer under `token`.
    #[must_use]
    pub fn path(&self, token: &str) -> String {
        format!("{PAGE}{}?grant={token}", self.id)
    }

    /// The page title.
    #[must_use]
    pub fn title(&self) -> String {
        format!("Set private zone `{}`", self.alias)
    }
}

/// The offers, kept by the `Instance`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Offers {
    /// The last offer's number.
    #[serde(default)]
    pub seq: u32,
    /// Offers by their token's key.
    #[serde(default)]
    pub offers: BTreeMap<String, Offer>,
}

/// Checks an alias.
///
/// # Errors
///
/// When it is empty or holds a byte outside `[a-z0-9-]`.
pub fn alias(a: &str) -> Result<(), String> {
    if !a.is_empty()
        && a.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Ok(())
    } else {
        Err("an alias is [a-z0-9-]+, with no dot".into())
    }
}

/// Reads an entry from its JSON.
///
/// # Errors
///
/// When it is not an object of `zone`, `services` and `records`, or one is malformed.
pub fn entry(v: &Value) -> Result<Entry, String> {
    let o = v
        .as_object()
        .ok_or("an entry takes only zone, services and records")?;
    if o.keys()
        .any(|k| !["zone", "services", "records"].contains(&k.as_str()))
    {
        return Err("an entry takes only zone, services and records".into());
    }
    let zone = o
        .get("zone")
        .and_then(Value::as_str)
        .filter(|z| z.contains('.'))
        .ok_or("an entry needs its zone, a name with a dot")?;
    let map = |k: &str| -> Result<Map<String, Value>, String> {
        match o.get(k) {
            None => Ok(Map::new()),
            Some(Value::Object(m)) => Ok(m.clone()),
            Some(_) => Err(format!("{k} must be an object")),
        }
    };
    Ok(Entry {
        zone: zone.to_string(),
        services: map("services")?,
        records: map("records")?,
        updated: 0,
    })
}

impl Offers {
    /// Offers `draft` (its `id` and `expires` set here) under `token` until `expires`, dropping
    /// expired offers.
    ///
    /// # Errors
    ///
    /// When the alias or the entry's zone is malformed.
    pub fn offer(
        &mut self,
        mut draft: Offer,
        token: &str,
        now: u64,
        expires: u64,
    ) -> Result<Offer, String> {
        alias(&draft.alias)?;
        if !draft.entry.zone.contains('.') {
            return Err("an entry needs its zone, a name with a dot".into());
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

/// The offer number in a page path `/zone-private/<id>`.
#[must_use]
pub fn parse(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PAGE)?;
    let id = rest.split_once('?').map_or(rest, |(p, _)| p);
    (!id.starts_with('0')).then(|| id.parse().ok()).flatten()
}

/// Stores the offer's entry under its alias, stamped `now`.
pub fn set(store: &mut Store, offer: &Offer, now: u64) {
    let mut e = offer.entry.clone();
    e.updated = now;
    store.insert(offer.alias.clone(), e);
}

/// The aliases and when each was set; never a zone.
#[must_use]
pub fn listing(store: &Store) -> Value {
    let zones: Vec<Value> = store
        .iter()
        .map(|(a, e)| json!({"alias": a, "updated": e.updated}))
        .collect();
    json!({ "zones": zones })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_000_000;

    fn draft(alias: &str, zone: &str) -> Offer {
        Offer {
            id: 0,
            alias: alias.into(),
            entry: Entry {
                zone: zone.into(),
                services: Map::new(),
                records: Map::new(),
                updated: 0,
            },
            expires: 0,
        }
    }

    #[test]
    fn an_alias_has_no_dot() {
        assert!(alias("blog").is_ok());
        assert!(alias("blog-2").is_ok());
        assert!(alias("a.b").is_err());
        assert!(alias("").is_err());
        assert!(alias("Blog").is_err());
    }

    #[test]
    fn an_entry_needs_its_zone_and_objects() {
        let e = entry(&json!({"zone": "example.com"})).unwrap();
        assert!(e.services.is_empty() && e.records.is_empty());
        assert!(entry(&json!({"zone": "example"})).is_err());
        assert!(entry(&json!({})).is_err());
        assert!(entry(&json!({"zone": "example.com", "records": []})).is_err());
        assert!(entry(&json!({"zone": "example.com", "services": "x"})).is_err());
        assert!(entry(&json!({"zone": "example.com", "kind": 1})).is_err());
    }

    #[test]
    fn an_offer_is_numbered_and_taken_once() {
        let mut offers = Offers::default();
        let o = offers
            .offer(draft("blog", "example.com"), "tok", NOW, NOW + 60)
            .unwrap();
        assert_eq!(o.path("tok"), "/zone-private/1?grant=tok");
        assert_eq!(o.title(), "Set private zone `blog`");
        assert!(offers.take("tok", NOW).is_ok());
        assert!(offers.take("tok", NOW).is_err());
        assert!(
            offers
                .offer(draft("a.b", "example.com"), "t2", NOW, NOW + 60)
                .is_err()
        );
    }

    #[test]
    fn a_set_stamps_updated_and_the_listing_names_no_zone() {
        let mut store = Store::new();
        let o = draft("blog", "example.com");
        set(&mut store, &o, NOW);
        assert_eq!(store["blog"].updated, NOW);
        let l = listing(&store);
        assert_eq!(l, json!({"zones": [{"alias": "blog", "updated": NOW}]}));
        assert!(!l.to_string().contains("example.com"));
    }

    #[test]
    fn a_page_path_names_its_offer() {
        assert_eq!(parse("/zone-private/7?grant=x"), Some(7));
        assert_eq!(parse("/zone-private/07"), None);
        assert_eq!(parse("/zone-private/"), None);
        assert_eq!(parse("/member-grant/7"), None);
    }

    #[test]
    fn the_page_shows_the_zone_and_the_proof_view_never_does() {
        let o = draft("blog", "example.com");
        let page = crate::approve::zone_private_page(&o, "tok");
        assert!(page.contains("example.com"));
        assert!(page.contains("/zone-private/"));
        let asked = crate::present::asked::zone_private(&o).to_string();
        assert!(asked.contains("blog"));
        assert!(!asked.contains("example.com"));
    }
}
