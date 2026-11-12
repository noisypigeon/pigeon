//! The app's devices: linked with the owner's proof, then authenticated by an ES256 signature
//! over every request; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §2.

use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::core::approval::passkeys::point;
use crate::push::{b64url, b64url_decode};

/// How long a link code may be redeemed, in seconds.
pub const LINK_TTL: u64 = 2 * 60;
/// How far a request's timestamp may be from the Worker's clock, in seconds.
pub const SKEW: u64 = 60;
/// A device unused this long is unlinked, in seconds.
pub const IDLE: u64 = 30 * 24 * 60 * 60;
/// The request header carrying the device's signature.
pub const HEADER: &str = "x-pigeon-device";
/// The longest device name kept.
const NAME_MAX: usize = 64;

/// A linked device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// Its ID: its key's fingerprint.
    pub id: String,
    /// The name it gave.
    pub name: String,
    /// Its public key, the uncompressed P-256 point, base64url.
    pub key: String,
    /// When it was linked.
    pub linked: u64,
    /// When it last made a signed request.
    pub seen: u64,
    /// Its APNs token, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<String>,
}

/// A link the owner proved, waiting for the device to redeem it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// The code's SHA-256, hex.
    pub code: String,
    /// The device's name.
    pub name: String,
    /// The device's key, the uncompressed point, base64url.
    pub key: String,
    /// When it lapses.
    pub expires: u64,
}

/// A signature seen recently, kept to refuse its replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    /// The signature's SHA-256, hex.
    pub sig: String,
    /// When it lapses from the window.
    pub until: u64,
}

/// An instance's devices, link offers and recent signatures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Devices {
    /// The linked devices.
    #[serde(default)]
    pub devices: Vec<Device>,
    /// Links proved and not yet redeemed.
    #[serde(default)]
    pub offers: Vec<Offer>,
    /// Signatures inside the replay window.
    #[serde(default)]
    pub seen: Vec<Seen>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A key's fingerprint: the first 16 bytes of its point's SHA-256, hex.
#[must_use]
pub fn fingerprint(point: &[u8]) -> String {
    hex(&Sha256::digest(point)[..16])
}

/// The uncompressed point of `key`, a P-256 `SubjectPublicKeyInfo` or a raw point, base64url.
///
/// # Errors
///
/// When it isn't a P-256 key.
pub fn key_point(key: &str) -> Result<Vec<u8>, String> {
    let bytes = b64url_decode(key).map_err(|_| "the key isn't base64url".to_string())?;
    let p = if bytes.len() == 65 {
        bytes
    } else {
        point(&bytes)?
    };
    VerifyingKey::from_sec1_bytes(&p).map_err(|_| "the key isn't a P-256 point".to_string())?;
    Ok(p)
}

/// A device name kept as given, trimmed, without control characters.
#[must_use]
pub fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).collect();
    let n = n.trim();
    if n.is_empty() {
        "iPhone".into()
    } else {
        n.chars().take(NAME_MAX).collect()
    }
}

/// Whether `signature`, raw `r‖s` or DER, base64url, is `point`'s ES256 signature of `data`.
#[must_use]
pub fn verifies(point: &[u8], data: &[u8], signature: &str) -> bool {
    let Ok(raw) = b64url_decode(signature) else {
        return false;
    };
    let sig = if raw.len() == 64 {
        Signature::from_slice(&raw).ok()
    } else {
        crate::core::approval::passkeys::der_signature(&raw).ok()
    };
    let Some(sig) = sig else {
        return false;
    };
    let sig = sig.normalize_s().unwrap_or(sig);
    VerifyingKey::from_sec1_bytes(point).is_ok_and(|k| k.verify(data, &sig).is_ok())
}

/// The text a device signs for one request.
#[must_use]
pub fn canonical(method: &str, path: &str, at: u64, body: &[u8]) -> String {
    format!(
        "pigeon-device-v1\n{}\n{path}\n{at}\n{}",
        method.to_ascii_uppercase(),
        sha256_hex(body)
    )
}

/// The text a device signs to redeem a link code.
#[must_use]
pub fn link_message(code: &str) -> String {
    format!("pigeon-link-v1\n{code}")
}

/// The header's three parts: `id=…;t=…;s=…`.
///
/// # Errors
///
/// When a part is missing or malformed.
pub fn parse_header(header: &str) -> Result<(String, u64, String), String> {
    let (mut id, mut at, mut sig) = (None, None, None);
    for part in header.split(';') {
        match part.trim().split_once('=') {
            Some(("id", v)) => id = Some(v.to_string()),
            Some(("t", v)) => at = v.parse::<u64>().ok(),
            Some(("s", v)) => sig = Some(v.to_string()),
            _ => {}
        }
    }
    match (id, at, sig) {
        (Some(i), Some(t), Some(s)) if !i.is_empty() && !s.is_empty() => Ok((i, t, s)),
        _ => Err(format!("`{HEADER}` must be id=…;t=…;s=…")),
    }
}

impl Devices {
    /// Drops lapsed offers and signatures, and devices idle past [`IDLE`].
    #[must_use]
    pub fn prune(mut self, now: u64) -> Self {
        self.offers.retain(|o| o.expires > now);
        self.seen.retain(|s| s.until > now);
        self.devices.retain(|d| d.seen.max(d.linked) + IDLE > now);
        self
    }

    /// Records the owner's proved link of the device with `key`, named `name`, redeemable once
    /// with `code` until [`LINK_TTL`] from `now`.
    ///
    /// # Errors
    ///
    /// When the key isn't a P-256 key.
    pub fn offer(self, name: &str, key: &str, code: &str, now: u64) -> Result<Self, String> {
        let p = key_point(key)?;
        let mut d = self.prune(now);
        let key = b64url(&p);
        d.offers.retain(|o| o.key != key);
        d.offers.push(Offer {
            code: sha256_hex(code.as_bytes()),
            name: clean_name(name),
            key,
            expires: now + LINK_TTL,
        });
        Ok(d)
    }

    /// Redeems `code` for the device holding `key`, proved by `signature` over
    /// [`link_message`]: the linked device. The offer is used up even when refused.
    pub fn redeem(
        self,
        code: &str,
        key: &str,
        signature: &str,
        now: u64,
    ) -> (Self, Result<Device, String>) {
        let mut d = self.prune(now);
        let hash = sha256_hex(code.as_bytes());
        let Some(at) = d.offers.iter().position(|o| o.code == hash) else {
            return (d, Err("no such link, or it has lapsed".into()));
        };
        let offer = d.offers.remove(at);
        let Ok(p) = key_point(key) else {
            return (d, Err("the key isn't a P-256 key".into()));
        };
        if b64url(&p) != offer.key {
            return (d, Err("the link was proved for another key".into()));
        }
        if !verifies(&p, link_message(code).as_bytes(), signature) {
            return (d, Err("the signature over the code doesn't verify".into()));
        }
        let device = Device {
            id: fingerprint(&p),
            name: offer.name,
            key: offer.key,
            linked: now,
            seen: now,
            push: None,
        };
        d.devices.retain(|x| x.id != device.id);
        d.devices.push(device.clone());
        (d, Ok(device))
    }

    /// Checks a request's `header` over its `method`, `path` and `body` at `now`: the device that
    /// signed it. A refused signature is not recorded; an accepted one can't be replayed.
    pub fn check(
        self,
        header: &str,
        method: &str,
        path: &str,
        body: &[u8],
        now: u64,
    ) -> (Self, Result<Device, String>) {
        let mut d = self.prune(now);
        let (id, at, sig) = match parse_header(header) {
            Ok(x) => x,
            Err(e) => return (d, Err(e)),
        };
        if at.abs_diff(now) > SKEW {
            return (
                d,
                Err("the request's time is too far from the Worker's".into()),
            );
        }
        let Some(i) = d.devices.iter().position(|x| x.id == id) else {
            return (d, Err("no such device; link it again".into()));
        };
        let Ok(p) = key_point(&d.devices[i].key) else {
            return (d, Err("the device's key is unreadable".into()));
        };
        if !verifies(&p, canonical(method, path, at, body).as_bytes(), &sig) {
            return (d, Err("the request's signature doesn't verify".into()));
        }
        let seen = sha256_hex(sig.as_bytes());
        if d.seen.iter().any(|s| s.sig == seen) {
            return (d, Err("the request was replayed".into()));
        }
        d.seen.push(Seen {
            sig: seen,
            until: now + 2 * SKEW,
        });
        d.devices[i].seen = now;
        let device = d.devices[i].clone();
        (d, Ok(device))
    }

    /// Unlinks device `id`.
    #[must_use]
    pub fn unlink(mut self, id: &str) -> Self {
        self.devices.retain(|d| d.id != id);
        self
    }

    /// Sets device `id`'s APNs token.
    #[must_use]
    pub fn set_push(mut self, id: &str, token: Option<String>) -> Self {
        if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
            d.push = token;
        }
        self
    }
}
