//! Accounts and profiles: who an instance's devices belong to; see
//! `.claude/decisions/2026-11-14-runtime-accounts-profiles-instances.md`.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::core::app::devices::Devices;

#[cfg(test)]
mod tests;

/// The Instance storage key holding [`Accounts`].
pub const STORE: &str = "accounts";
/// An account ID's prefix.
pub const PREFIX: &str = "acct_";
/// The owner's profile name until they set one.
pub const DEFAULT_NAME: &str = "Owner";
/// The largest avatar kept, in bytes.
pub const AVATAR_MAX: usize = 64 * 1024;
/// The fewest passkeys an account keeps.
pub const MIN_PASSKEYS: usize = 1;
/// The ID alphabet.
pub const CROCKFORD: &str = "0123456789abcdefghjkmnpqrstvwxyz";
/// The longest name kept.
const NAME_MAX: usize = 64;

/// What an account is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A person.
    Person,
    /// A service.
    Service,
}

/// How the app is drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Follow the device.
    #[default]
    System,
    /// Light.
    Light,
    /// Dark.
    Dark,
}

/// An account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Its ID, `acct_` and 26 characters.
    pub id: String,
    /// What it is.
    pub kind: Kind,
    /// When it was created.
    pub created: u64,
}

/// An account's profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Its account's ID.
    pub account: String,
    /// The name shown.
    pub name: String,
    /// How the app is drawn.
    pub appearance: Appearance,
    /// The avatar's SHA-256, hex, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// When it last changed.
    pub updated: u64,
}

/// An instance's accounts and their profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accounts {
    /// The accounts.
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Their profiles.
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// An account ID from `random_hex`: its first 32 hex characters as 16 bytes, Crockford base32.
#[must_use]
pub fn account_id(random_hex: &str) -> String {
    let digits: Vec<u8> = random_hex
        .chars()
        .take(32)
        .map(|c| {
            c.to_digit(16)
                .and_then(|d| u8::try_from(d).ok())
                .unwrap_or(0)
        })
        .collect();
    let mut bytes = [0u8; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        let hi = digits.get(2 * i).copied().unwrap_or(0);
        let lo = digits.get(2 * i + 1).copied().unwrap_or(0);
        *b = (hi << 4) | lo;
    }
    let alphabet = CROCKFORD.as_bytes();
    let mut out = String::from(PREFIX);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(alphabet[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(alphabet[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The storage key of `account`'s avatar bytes, base64url.
#[must_use]
pub fn avatar_key(account: &str) -> String {
    format!("avatar:{account}")
}

/// A name kept trimmed, without control characters, at most 64 characters.
///
/// # Errors
///
/// When nothing is left.
pub fn clean_name(name: &str) -> Result<String, String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect();
    let n = n.trim();
    if n.is_empty() {
        return Err("the name is empty".into());
    }
    Ok(n.chars().take(NAME_MAX).collect())
}

/// `s` as an [`Appearance`].
///
/// # Errors
///
/// When it isn't `system`, `light` or `dark`.
pub fn parse_appearance(s: &str) -> Result<Appearance, String> {
    match s {
        "system" => Ok(Appearance::System),
        "light" => Ok(Appearance::Light),
        "dark" => Ok(Appearance::Dark),
        _ => Err("appearance is system, light or dark".into()),
    }
}

/// Checks `bytes` is a JPEG no larger than [`AVATAR_MAX`].
///
/// # Errors
///
/// When it isn't, or is too large.
pub fn check_jpeg(bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err("the avatar isn't a JPEG".into());
    }
    if bytes.len() > AVATAR_MAX {
        return Err("the avatar is over 64 KiB".into());
    }
    Ok(())
}

/// `devices` with `account` set on each device that has none.
#[must_use]
pub fn stamp(mut devices: Devices, account: &str) -> Devices {
    for d in &mut devices.devices {
        if d.account.is_empty() {
            d.account = account.to_string();
        }
    }
    devices
}

/// Whether a passkey may be removed when `count` are enrolled.
///
/// # Errors
///
/// When it would leave fewer than [`MIN_PASSKEYS`].
pub fn may_remove_passkey(count: usize) -> Result<(), String> {
    if count <= MIN_PASSKEYS {
        return Err("the last passkey can't be removed".into());
    }
    Ok(())
}

/// The passkey purpose for unlinking device `id`.
#[must_use]
pub fn unlink_purpose(id: &str) -> String {
    format!("unlink:{id}")
}

/// The passkey purpose for removing passkey `id`.
#[must_use]
pub fn passkey_remove_purpose(id: &str) -> String {
    format!("passkey:remove:{id}")
}

impl Accounts {
    /// The first person's account, created with its profile when there is none.
    #[must_use]
    pub fn ensure_owner(mut self, random_hex: &str, now: u64) -> (Self, Account) {
        let account = if let Some(a) = self.accounts.iter().find(|a| a.kind == Kind::Person) {
            a.clone()
        } else {
            let a = Account {
                id: account_id(random_hex),
                kind: Kind::Person,
                created: now,
            };
            self.accounts.push(a.clone());
            a
        };
        if self.profile(&account.id).is_none() {
            self.profiles.push(Profile {
                account: account.id.clone(),
                name: DEFAULT_NAME.into(),
                appearance: Appearance::System,
                avatar: None,
                updated: now,
            });
        }
        (self, account)
    }

    /// `account`'s profile.
    #[must_use]
    pub fn profile(&self, account: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.account == account)
    }

    fn edit(
        mut self,
        account: &str,
        now: u64,
        f: impl FnOnce(&mut Profile) -> Result<(), String>,
    ) -> Result<(Self, Profile), String> {
        let Some(p) = self.profiles.iter_mut().find(|p| p.account == account) else {
            return Err("no such profile".into());
        };
        f(p)?;
        p.updated = now;
        let p = p.clone();
        Ok((self, p))
    }

    /// Sets `account`'s name and appearance, each when given.
    ///
    /// # Errors
    ///
    /// When the profile is unknown or a value is refused.
    pub fn update(
        self,
        account: &str,
        name: Option<&str>,
        appearance: Option<&str>,
        now: u64,
    ) -> Result<(Self, Profile), String> {
        let name = name.map(clean_name).transpose()?;
        let appearance = appearance.map(parse_appearance).transpose()?;
        self.edit(account, now, |p| {
            if let Some(n) = name {
                p.name = n;
            }
            if let Some(a) = appearance {
                p.appearance = a;
            }
            Ok(())
        })
    }

    /// Sets `account`'s avatar to `jpeg`'s SHA-256.
    ///
    /// # Errors
    ///
    /// When the profile is unknown or `jpeg` is refused.
    pub fn set_avatar(
        self,
        account: &str,
        jpeg: &[u8],
        now: u64,
    ) -> Result<(Self, Profile), String> {
        check_jpeg(jpeg)?;
        let sum = hex(&Sha256::digest(jpeg));
        self.edit(account, now, |p| {
            p.avatar = Some(sum);
            Ok(())
        })
    }

    /// Removes `account`'s avatar.
    ///
    /// # Errors
    ///
    /// When the profile is unknown.
    pub fn clear_avatar(self, account: &str, now: u64) -> Result<(Self, Profile), String> {
        self.edit(account, now, |p| {
            p.avatar = None;
            Ok(())
        })
    }
}
