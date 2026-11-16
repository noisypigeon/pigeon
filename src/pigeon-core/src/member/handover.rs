//! An app-shape standalone's hand-over: the owner's account, passkeys and devices, public keys
//! only, seeded once into an empty `Instance`; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`, its 2026-11-15
//! amendment.

use serde::{Deserialize, Serialize};

use super::Members;
use crate::account::{Account, Accounts, Appearance, DEFAULT_NAME, Kind, Profile};
use crate::app::devices::{Device, Devices};
use crate::approval::passkeys::{Keyring, Passkey};

/// A passkey handed over: its credential ID and public key, both base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandedPasskey {
    /// Its credential ID.
    pub id: String,
    /// The uncompressed P-256 point.
    pub key: String,
    /// What the owner called it.
    pub name: String,
    /// Whether it is synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<bool>,
}

/// A device handed over: its ID, name and public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandedDevice {
    /// Its ID, the key's fingerprint.
    pub id: String,
    /// The name it gave.
    pub name: String,
    /// The uncompressed P-256 point, base64url.
    pub key: String,
}

/// What `HANDOVER` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handover {
    /// The owner's account ID on the platform.
    pub owner: String,
    /// Their passkeys.
    pub passkeys: Vec<HandedPasskey>,
    /// Their devices.
    pub devices: Vec<HandedDevice>,
}

/// The four stores a hand-over seeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seeded {
    /// Accounts and profiles.
    pub accounts: Accounts,
    /// Memberships.
    pub members: Members,
    /// Linked devices.
    pub devices: Devices,
    /// Passkeys.
    pub ring: Keyring,
}

impl Seeded {
    /// Whether nothing is stored yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.accounts.accounts.is_empty()
            && self.members.memberships.is_empty()
            && self.devices.devices.is_empty()
            && self.ring.keys.is_empty()
    }
}

impl Handover {
    /// `text` parsed and checked.
    ///
    /// # Errors
    /// Not JSON of this shape, an owner that is not an account ID, or no passkey or device.
    pub fn parse(text: &str) -> Result<Self, String> {
        let h: Self = serde_json::from_str(text).map_err(|e| format!("HANDOVER: {e}"))?;
        let id_ok = h
            .owner
            .strip_prefix(crate::account::PREFIX)
            .is_some_and(|r| r.len() == 26 && r.bytes().all(|b| b.is_ascii_alphanumeric()));
        if !id_ok {
            return Err("HANDOVER: `owner` is not an account ID".into());
        }
        if h.passkeys.is_empty() || h.devices.is_empty() {
            return Err("HANDOVER: it needs a passkey and a device".into());
        }
        Ok(h)
    }

    /// `stores` seeded with the owner as the first administrator, when all four are empty;
    /// otherwise unchanged. The second value says whether it seeded.
    #[must_use]
    pub fn seed(&self, stores: Seeded, now: u64) -> (Seeded, bool) {
        if !stores.is_empty() {
            return (stores, false);
        }
        let mut s = stores;
        s.accounts.accounts.push(Account {
            id: self.owner.clone(),
            kind: Kind::Person,
            created: now,
        });
        s.accounts.profiles.push(Profile {
            account: self.owner.clone(),
            name: DEFAULT_NAME.into(),
            appearance: Appearance::System,
            avatar: None,
            updated: now,
        });
        s.members = s.members.seed(&self.owner, now);
        s.devices.devices = self
            .devices
            .iter()
            .map(|d| Device {
                id: d.id.clone(),
                name: d.name.clone(),
                key: d.key.clone(),
                linked: now,
                seen: 0,
                push: None,
                account: self.owner.clone(),
            })
            .collect();
        s.ring.keys = self
            .passkeys
            .iter()
            .map(|p| Passkey {
                id: p.id.clone(),
                key: p.key.clone(),
                counter: 0,
                name: p.name.clone(),
                created: now,
                used: None,
                synced: p.synced,
                failures: Vec::new(),
            })
            .collect();
        (s, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member::Role;

    const OWNER: &str = "acct_0123456789abcdefghjkmnpqrs";

    fn handover() -> Handover {
        Handover::parse(&format!(
            r#"{{"owner":"{OWNER}","passkeys":[{{"id":"cred","key":"pk","name":"Phone","synced":true}}],"devices":[{{"id":"dev","name":"iPhone","key":"dk"}}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn it_seeds_the_owner_as_administrator_with_their_keys() {
        let (s, seeded) = handover().seed(Seeded::default(), 7);
        assert!(seeded);
        assert_eq!(s.accounts.accounts[0].id, OWNER);
        let me = s.members.active(OWNER).unwrap();
        assert_eq!(me.role, Role::Administrator);
        assert_eq!(s.devices.devices[0].account, OWNER);
        assert_eq!(s.devices.devices[0].key, "dk");
        assert_eq!(s.ring.keys[0].id, "cred");
        assert_eq!(s.ring.keys[0].synced, Some(true));
    }

    #[test]
    fn it_seeds_once_and_never_over_what_is_stored() {
        let (first, _) = handover().seed(Seeded::default(), 7);
        let mut other = handover();
        other.owner = "acct_zzzzzzzzzzzzzzzzzzzzzzzzzz".into();
        let (again, seeded) = other.seed(first.clone(), 9);
        assert!(!seeded);
        assert_eq!(again, first);
        let mut partial = Seeded::default();
        partial.ring.keys = first.ring.keys.clone();
        let (kept, seeded) = handover().seed(partial.clone(), 9);
        assert!(!seeded);
        assert_eq!(kept, partial);
    }

    #[test]
    fn parse_refuses_a_secret_shaped_or_empty_handover() {
        assert!(Handover::parse("{}").is_err());
        assert!(Handover::parse(r#"{"owner":"acct_x","passkeys":[],"devices":[]}"#).is_err());
        let extra = format!(
            r#"{{"owner":"{OWNER}","passkeys":[{{"id":"c","key":"k","name":"n"}}],"devices":[{{"id":"d","name":"n","key":"k"}}],"token":"t"}}"#
        );
        assert!(Handover::parse(&extra).is_err());
    }
}
