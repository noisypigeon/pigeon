//! Joining a standalone: the invitee's app registers a passkey for the platform's relying party
//! and offers its device key under the invitation's token; an administrator admits it after
//! comparing the match code; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`, its 2026-11-15
//! amendment, §7.

use serde::{Deserialize, Serialize};

use super::{Invitation, InviteStatus, Members, token_id};
use crate::account::{Account, Accounts, Appearance, Kind, Profile, account_id};
use crate::app::devices::{Device, Devices, clean_name, fingerprint, key_point, verifies};
use crate::approval::passkeys::register;
use crate::push::b64url;

/// Where the `Instance` keeps joins.
pub const STORE: &str = "members.joins";

/// How long a join challenge lasts, in seconds.
pub const CHALLENGE_TTL: u64 = 300;

/// The passkey purpose that proves an invitation of `email`.
#[must_use]
pub fn invite_purpose(email: &str) -> String {
    format!("member:invite:{}", email.trim().to_lowercase())
}

/// The passkey purpose that proves admitting invitation `id`.
#[must_use]
pub fn admit_purpose(id: &str) -> String {
    format!("member:admit:{id}")
}

/// The text the invitee's device signs to join with `token`.
#[must_use]
pub fn join_message(token: &str) -> String {
    format!("pigeon-join-v1\n{token}")
}

/// A join challenge, for one invitation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    /// The challenge, base64url.
    pub challenge: String,
    /// The invitation's ID.
    pub invitation: String,
    /// When it lapses.
    pub expires: u64,
}

/// A passkey an invitee registered, kept for their later proofs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberPasskey {
    /// Its credential ID, base64url.
    pub id: String,
    /// The uncompressed P-256 point, base64url.
    pub key: String,
    /// When it was registered.
    pub created: u64,
}

/// An enrolled invitee waiting for an administrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    /// The invitation's ID.
    pub invitation: String,
    /// Its account.
    pub account: String,
    /// The device it will link.
    pub device: Device,
    /// Its passkey.
    pub passkey: MemberPasskey,
}

/// Joins in progress, and the passkeys of those admitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joins {
    /// Live challenges.
    #[serde(default)]
    pub challenges: Vec<Challenge>,
    /// Enrolled, not yet admitted.
    #[serde(default)]
    pub waiting: Vec<Waiting>,
    /// Each admitted account's passkeys.
    #[serde(default)]
    pub passkeys: Vec<(String, MemberPasskey)>,
}

/// What the invitee's app sends to join.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    /// The invitation's token.
    pub token: String,
    /// The name to show.
    #[serde(default)]
    pub name: String,
    /// The device's name.
    #[serde(default)]
    pub device_name: String,
    /// The passkey's credential ID, base64url.
    pub passkey_id: String,
    /// Its `clientDataJSON`, base64url.
    pub client: String,
    /// Its authenticator data, base64url.
    pub authenticator: String,
    /// Its public key, a P-256 `SubjectPublicKeyInfo`, base64url.
    pub spki: String,
    /// The device's public key, base64url.
    pub device_key: String,
    /// The device's signature of [`join_message`], base64url.
    pub device_signature: String,
}

fn bytes(s: &str) -> Vec<u8> {
    crate::push::b64url_decode(s).unwrap_or_default()
}

fn open_invitation<'a>(m: &'a Members, token: &str, now: u64) -> Result<&'a Invitation, String> {
    let id = token_id(token);
    let inv = m
        .invitations
        .iter()
        .find(|i| i.id == id)
        .ok_or("no such invitation")?;
    if inv.status != InviteStatus::Open {
        return Err("this link was already used".into());
    }
    if inv.expires <= now {
        return Err("this link has expired".into());
    }
    Ok(inv)
}

impl Joins {
    /// A challenge for the invitation `token` opens: the next joins, and the challenge.
    ///
    /// # Errors
    /// No such open invitation.
    pub fn challenge(
        mut self,
        members: &Members,
        token: &str,
        random: &[u8],
        now: u64,
    ) -> Result<(Self, String), String> {
        let inv = open_invitation(members, token, now)?;
        self.challenges.retain(|c| c.expires > now);
        let challenge = b64url(random);
        self.challenges.push(Challenge {
            challenge: challenge.clone(),
            invitation: inv.id.clone(),
            expires: now + CHALLENGE_TTL,
        });
        Ok((self, challenge))
    }

    /// Enrols `req`: its passkey's registration (from `host`, for relying party `rp`) and its
    /// device's signature checked, a `person` account made from `random_hex`, the invitation
    /// enrolled, and the device held until an administrator admits it. The challenge is used up
    /// even when the join is refused.
    ///
    /// # Errors
    /// Saying why it is refused.
    #[allow(clippy::too_many_arguments)]
    pub fn join(
        mut self,
        members: Members,
        accounts: Accounts,
        req: &JoinRequest,
        host: &str,
        rp: &str,
        random_hex: &str,
        now: u64,
    ) -> (Self, Result<(Members, Accounts, Invitation), String>) {
        let client = bytes(&req.client);
        let answered = serde_json::from_slice::<serde_json::Value>(&client)
            .ok()
            .and_then(|v| v["challenge"].as_str().map(str::to_string))
            .unwrap_or_default();
        let id = token_id(&req.token);
        let Some(at) = self
            .challenges
            .iter()
            .position(|c| c.challenge == answered && c.invitation == id && c.expires > now)
        else {
            return (
                self,
                Err("this passkey answered no challenge for this link".into()),
            );
        };
        self.challenges.remove(at);
        let outcome = (|| {
            open_invitation(&members, &req.token, now)?;
            let (point, _) = register(
                &client,
                &bytes(&req.authenticator),
                &bytes(&req.spki),
                &answered,
                host,
                rp,
            )?;
            if req.passkey_id.is_empty() {
                return Err("the passkey has no ID".to_string());
            }
            let device = key_point(&req.device_key)?;
            if !verifies(
                &device,
                join_message(&req.token).as_bytes(),
                &req.device_signature,
            ) {
                return Err("the device's signature over the link doesn't verify".to_string());
            }
            let account = account_id(random_hex);
            let (members, inv) =
                members.enrol(&req.token, &account, &req.passkey_id, &point, now)?;
            let mut accounts = accounts;
            accounts.accounts.push(Account {
                id: account.clone(),
                kind: Kind::Person,
                created: now,
            });
            let name: String = req
                .name
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect();
            accounts.profiles.push(Profile {
                account: account.clone(),
                name: if name.is_empty() {
                    "Member".into()
                } else {
                    name
                },
                appearance: Appearance::System,
                avatar: None,
                updated: now,
            });
            let waiting = Waiting {
                invitation: inv.id.clone(),
                account: account.clone(),
                device: Device {
                    id: fingerprint(&device),
                    name: clean_name(&req.device_name),
                    key: b64url(&device),
                    linked: 0,
                    seen: 0,
                    push: None,
                    account,
                },
                passkey: MemberPasskey {
                    id: req.passkey_id.clone(),
                    key: b64url(&point),
                    created: now,
                },
            };
            Ok((members, accounts, inv, waiting))
        })();
        match outcome {
            Ok((members, accounts, inv, waiting)) => {
                self.waiting.retain(|w| w.invitation != inv.id);
                self.waiting.push(waiting);
                (self, Ok((members, accounts, inv)))
            }
            Err(why) => (self, Err(why)),
        }
    }

    /// `by` admits invitation `id`: its membership made, its device linked, its passkey kept.
    ///
    /// # Errors
    /// As [`Members::admit`], or nothing is waiting under `id`.
    pub fn admit(
        mut self,
        members: Members,
        devices: Devices,
        id: &str,
        by: &str,
        now: u64,
    ) -> Result<(Self, Members, Devices), String> {
        let at = self
            .waiting
            .iter()
            .position(|w| w.invitation == id)
            .ok_or("nothing is waiting under this invitation")?;
        let (members, _) = members.admit(id, by, now)?;
        let w = self.waiting.remove(at);
        let mut devices = devices;
        devices.devices.retain(|d| d.id != w.device.id);
        devices.devices.push(Device {
            linked: now,
            ..w.device
        });
        self.passkeys.push((w.account, w.passkey));
        Ok((self, members, devices))
    }

    /// Invitation `id`'s state as the invitee's app asks it: `open`, `enrolled`, `admitted`,
    /// `declined`, `revoked` or `expired`.
    #[must_use]
    pub fn state(members: &Members, id: &str, now: u64) -> Option<&'static str> {
        let inv = members.invitations.iter().find(|i| i.id == id)?;
        Some(match inv.status {
            InviteStatus::Open | InviteStatus::Enrolled if inv.expires <= now => "expired",
            InviteStatus::Open => "open",
            InviteStatus::Enrolled => "enrolled",
            InviteStatus::Admitted => "admitted",
            InviteStatus::Declined => "declined",
            InviteStatus::Revoked => "revoked",
            InviteStatus::Expired => "expired",
        })
    }
}

#[cfg(test)]
#[path = "join/tests.rs"]
mod tests;
