//! Passkeys: `WebAuthn` ES256 credentials that prove the owner beside the authenticator code; see
//! `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §4.

use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// How long a challenge lives, in seconds.
pub const CHALLENGE_TTL: u64 = 5 * 60;

/// A P-256 public key's DER `SubjectPublicKeyInfo`, before its 65-byte uncompressed point.
const SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The authenticator data's user-present flag.
const USER_PRESENT: u8 = 0x01;
/// The authenticator data's user-verified flag.
const USER_VERIFIED: u8 = 0x04;
/// The authenticator data's backup-eligible flag: the passkey is synced.
const BACKUP_ELIGIBLE: u8 = 0x08;

/// The most passkeys a keyring holds; see
/// `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.
pub const MAX_KEYS: usize = 8;

/// How many refused assertions lock a passkey, and for how long, in seconds; see that ADR's §2.
pub const LOCK_AFTER: usize = 5;
/// The window those refusals are counted in.
pub const LOCK_WINDOW: u64 = 60 * 60;

/// A registered passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passkey {
    /// Its credential ID, base64url.
    pub id: String,
    /// Its public key, the uncompressed P-256 point, base64url.
    pub key: String,
    /// Its signature counter; synced passkeys stay at 0.
    #[serde(default)]
    pub counter: u32,
    /// What the owner called it.
    pub name: String,
    /// When it was registered, in Unix seconds.
    pub created: u64,
    /// When it last proved the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    /// Whether it is synced (backup-eligible) rather than device-bound; unknown for older ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<bool>,
    /// When its bound assertions were refused, within [`LOCK_WINDOW`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<u64>,
}

impl Passkey {
    /// Whether [`LOCK_AFTER`] refusals in the last [`LOCK_WINDOW`] lock it at `now`.
    #[must_use]
    pub fn locked(&self, now: u64) -> bool {
        self.failures
            .iter()
            .filter(|at| **at + LOCK_WINDOW > now)
            .count()
            >= LOCK_AFTER
    }
}

/// What the authenticator data says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthData {
    /// SHA-256 of the relying party's ID.
    pub rp_id_hash: [u8; 32],
    /// Its flags.
    pub flags: u8,
    /// Its signature counter.
    pub counter: u32,
}

/// Reads authenticator data's fixed head.
///
/// # Errors
///
/// When it is shorter than 37 bytes.
pub fn auth_data(bytes: &[u8]) -> Result<AuthData, String> {
    if bytes.len() < 37 {
        return Err("the authenticator data is too short".into());
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&bytes[..32]);
    Ok(AuthData {
        rp_id_hash,
        flags: bytes[32],
        counter: u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]),
    })
}

/// The uncompressed point of a P-256 `SubjectPublicKeyInfo`, as `getPublicKey()` gives it.
///
/// # Errors
///
/// When it isn't a P-256 key.
pub fn point(spki: &[u8]) -> Result<Vec<u8>, String> {
    if spki.len() != SPKI_PREFIX.len() + 65 || !spki.starts_with(&SPKI_PREFIX) {
        return Err("only P-256 (ES256) passkeys are accepted".into());
    }
    Ok(spki[SPKI_PREFIX.len()..].to_vec())
}

/// Checks `client_data` (`clientDataJSON`): its `type`, its challenge and its origin.
///
/// # Errors
///
/// Saying which doesn't match.
pub fn client_data(
    client_data: &[u8],
    kind: &str,
    challenge: &str,
    host: &str,
) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_slice(client_data).map_err(|_| "the client data isn't JSON")?;
    if v["type"] != kind {
        return Err(format!("this isn't a {kind}"));
    }
    if v["challenge"] != challenge {
        return Err("the challenge isn't this link's".into());
    }
    if v["origin"] != format!("https://{host}").as_str() {
        return Err("the passkey answered another site".into());
    }
    Ok(())
}

/// The relying party ID a page on `host` uses: `configured` (`PASSKEY_RP_ID`) when `host` is it
/// or under it, so one passkey serves every stack's hostname; else `host` itself.
#[must_use]
pub fn relying_party(host: &str, configured: Option<&str>) -> String {
    match configured.map(str::trim).filter(|rp| !rp.is_empty()) {
        Some(rp) if host == rp || host.ends_with(&format!(".{rp}")) => rp.to_string(),
        _ => host.to_string(),
    }
}

/// Checks the authenticator data's relying party, `rp`, and flags: presence and verification.
fn flags(data: &AuthData, rp: &str) -> Result<(), String> {
    if data.rp_id_hash[..] != Sha256::digest(rp.as_bytes())[..] {
        return Err("the passkey belongs to another site".into());
    }
    if data.flags & USER_PRESENT == 0 || data.flags & USER_VERIFIED == 0 {
        return Err("the passkey didn't verify you".into());
    }
    Ok(())
}

/// A registration's key and counter, once its client data (from `host`), authenticator data (for
/// relying party `rp`) and key check.
///
/// # Errors
///
/// Saying why it is refused.
pub fn register(
    client: &[u8],
    authenticator: &[u8],
    spki: &[u8],
    challenge: &str,
    host: &str,
    rp: &str,
) -> Result<(Vec<u8>, u32), String> {
    client_data(client, "webauthn.create", challenge, host)?;
    let data = auth_data(authenticator)?;
    flags(&data, rp)?;
    let key = point(spki)?;
    VerifyingKey::from_sec1_bytes(&key).map_err(|_| "the passkey's key isn't on P-256")?;
    Ok((key, data.counter))
}

/// An assertion's new counter, once its client data (from `host`), authenticator data (for `rp`),
/// counter and signature check against `passkey`.
///
/// # Errors
///
/// Saying why it is refused.
pub fn assert(
    passkey: &Passkey,
    client: &[u8],
    authenticator: &[u8],
    signature: &[u8],
    challenge: &str,
    host: &str,
    rp: &str,
) -> Result<u32, String> {
    client_data(client, "webauthn.get", challenge, host)?;
    let data = auth_data(authenticator)?;
    flags(&data, rp)?;
    if (passkey.counter > 0 || data.counter > 0) && data.counter <= passkey.counter {
        return Err("the passkey's counter went back; it may be cloned".into());
    }
    let key = crate::push::b64url_decode(&passkey.key)?;
    let key = VerifyingKey::from_sec1_bytes(&key).map_err(|_| "a stored key is unreadable")?;
    let mut signed = authenticator.to_vec();
    signed.extend_from_slice(&Sha256::digest(client));
    let sig = der_signature(signature)?;
    let sig = sig.normalize_s().unwrap_or(sig);
    key.verify(&signed, &sig)
        .map_err(|_| "the passkey's signature doesn't check".to_string())?;
    Ok(data.counter)
}

/// An ECDSA signature from its DER `SEQUENCE { INTEGER r, INTEGER s }`.
///
/// # Errors
///
/// When it isn't one.
pub fn der_signature(der: &[u8]) -> Result<Signature, String> {
    let bad = || "the signature isn't DER".to_string();
    let rest = match der {
        [0x30, len, rest @ ..] if usize::from(*len) == rest.len() => rest,
        _ => return Err(bad()),
    };
    let (r, rest) = der_integer(rest).ok_or_else(bad)?;
    let (s, rest) = der_integer(rest).ok_or_else(bad)?;
    if !rest.is_empty() {
        return Err(bad());
    }
    let mut bytes = [0u8; 64];
    bytes[32 - r.len()..32].copy_from_slice(r);
    bytes[64 - s.len()..].copy_from_slice(s);
    Signature::from_slice(&bytes).map_err(|_| bad())
}

/// One DER INTEGER's 32 or fewer value bytes, its leading zeros dropped, and what follows it.
fn der_integer(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let [0x02, len, rest @ ..] = der else {
        return None;
    };
    let len = usize::from(*len);
    if len == 0 || len > rest.len() {
        return None;
    }
    let (value, rest) = rest.split_at(len);
    let first = value.iter().position(|b| *b != 0).unwrap_or(value.len());
    let value = &value[first..];
    (value.len() <= 32).then_some((value, rest))
}

/// The constant user handle every passkey is registered under: there is one owner.
pub const USER: &[u8] = b"pigeon-owner";

/// What registering takes its challenge for.
pub const REGISTER: &str = "register";

/// The purpose a challenge for approval grant `token` is minted for; the token itself is never
/// kept.
#[must_use]
pub fn grant_purpose(token: &str) -> String {
    format!("grant:{}", super::grants::key(token))
}

/// What adding a passkey is proven for, by an existing one.
pub const ADD: &str = "passkey:add";

/// The purpose removing passkey `id` is proven for.
#[must_use]
pub fn remove_purpose(id: &str) -> String {
    format!("passkey:remove:{id}")
}

/// What a proof approves: the grant's operation, its target and the head or digest of what the
/// page shows; empty for a purpose without a grant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bound {
    /// The operation, such as `change.approve`.
    pub operation: String,
    /// What it acts on, such as a change's number.
    pub target: String,
    /// The head SHA, or the SHA-256 of what is approved.
    pub head: String,
}

/// The challenge a passkey signs for `purpose` and `bound` with `nonce`, base64url; see
/// `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §2.
#[must_use]
pub fn bound_challenge(purpose: &str, bound: &Bound, nonce: &[u8]) -> String {
    let mut h = Sha256::new();
    for part in [
        "pigeon-proof-v1",
        purpose,
        &bound.operation,
        &bound.target,
        &bound.head,
    ] {
        h.update(part.as_bytes());
        h.update(b"\n");
    }
    h.update(nonce);
    crate::push::b64url(&h.finalize())
}

/// What a bound proof leaves behind, each part base64url, for anyone holding the public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The passkey's credential ID.
    pub passkey: String,
    /// The challenge's nonce.
    pub nonce: String,
    /// The client data.
    pub client: String,
    /// The authenticator data.
    pub authenticator: String,
    /// The signature, DER.
    pub signature: String,
}

/// A challenge minted for one purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Minted {
    /// The challenge, base64url.
    pub challenge: String,
    /// What it may be used for: [`REGISTER`] or a [`grant_purpose`].
    pub purpose: String,
    /// When it lapses.
    pub expires: u64,
    /// A bound challenge's nonce, base64url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// The owner's passkeys and their open challenges, as the `Instance` keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyring {
    /// The registered passkeys.
    #[serde(default)]
    pub keys: Vec<Passkey>,
    /// Challenges not yet used.
    #[serde(default)]
    pub challenges: Vec<Minted>,
}

impl Keyring {
    /// Mints `random` as a challenge for `purpose`, dropping lapsed ones: the next keyring, and
    /// the challenge's base64url.
    #[must_use]
    pub fn mint(&self, purpose: &str, random: &[u8], now: u64) -> (Self, String) {
        let mut next = self.clone();
        next.challenges.retain(|c| c.expires > now);
        let challenge = crate::push::b64url(random);
        next.challenges.push(Minted {
            challenge: challenge.clone(),
            purpose: purpose.to_string(),
            expires: now + CHALLENGE_TTL,
            nonce: None,
        });
        (next, challenge)
    }

    /// Mints a challenge for `purpose` bound to `bound`, with `random` as its nonce, dropping
    /// lapsed ones: the next keyring, and the challenge's base64url.
    #[must_use]
    pub fn mint_bound(
        &self,
        purpose: &str,
        bound: &Bound,
        random: &[u8],
        now: u64,
    ) -> (Self, String) {
        let mut next = self.clone();
        next.challenges.retain(|c| c.expires > now);
        let challenge = bound_challenge(purpose, bound, random);
        next.challenges.push(Minted {
            challenge: challenge.clone(),
            purpose: purpose.to_string(),
            expires: now + CHALLENGE_TTL,
            nonce: Some(crate::push::b64url(random)),
        });
        (next, challenge)
    }

    /// Whether the challenge `client` answers was minted by [`Self::mint_bound`].
    #[must_use]
    pub fn bound(&self, client: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(client)
            .ok()
            .and_then(|v| v["challenge"].as_str().map(str::to_string))
            .is_some_and(|answered| {
                self.challenges
                    .iter()
                    .any(|c| c.challenge == answered && c.nonce.is_some())
            })
    }

    /// Whether passkey `id` is registered here.
    #[must_use]
    pub fn holds(&self, id: &str) -> bool {
        self.keys.iter().any(|k| k.id == id)
    }

    /// Uses up the challenge `client` answers, which must be for `purpose`: the keyring without
    /// it, and that challenge.
    fn take(&self, client: &[u8], purpose: &str, now: u64) -> Result<(Self, String), String> {
        let v: serde_json::Value =
            serde_json::from_slice(client).map_err(|_| "the client data isn't JSON")?;
        let answered = v["challenge"].as_str().unwrap_or_default().to_string();
        let at = self
            .challenges
            .iter()
            .position(|c| c.challenge == answered && c.purpose == purpose && c.expires > now)
            .ok_or("this passkey answered no challenge for this; reload the page")?;
        let mut next = self.clone();
        next.challenges.remove(at);
        Ok((next, answered))
    }

    /// Registers a passkey named `name` from a `webauthn.create` answer: the next keyring, whose
    /// challenge is used up even when the answer is refused, and why it was refused.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &self,
        id: &str,
        name: &str,
        client: &[u8],
        authenticator: &[u8],
        spki: &[u8],
        host: &str,
        rp: &str,
        now: u64,
    ) -> (Self, Result<(), String>) {
        let (mut next, challenge) = match self.take(client, REGISTER, now) {
            Ok(taken) => taken,
            Err(why) => return (self.clone(), Err(why)),
        };
        let (key, counter) = match register(client, authenticator, spki, &challenge, host, rp) {
            Ok(made) => made,
            Err(why) => return (next, Err(why)),
        };
        if id.is_empty() || next.keys.iter().any(|k| k.id == id) {
            return (next, Err("this passkey is already registered".into()));
        }
        if next.keys.len() >= MAX_KEYS {
            return (
                next,
                Err(format!("at most {MAX_KEYS} passkeys; remove one first")),
            );
        }
        let synced = auth_data(authenticator)
            .ok()
            .map(|d| d.flags & BACKUP_ELIGIBLE != 0);
        next.keys.push(Passkey {
            id: id.to_string(),
            key: crate::push::b64url(&key),
            counter,
            name: name.trim().chars().take(64).collect(),
            created: now,
            used: None,
            synced,
            failures: Vec::new(),
        });
        (next, Ok(()))
    }

    /// Checks a `webauthn.get` answer from passkey `id` for `purpose`, keeping its counter: the
    /// next keyring, whose challenge is used up even when the answer is refused, and why it was
    /// refused.
    #[allow(clippy::too_many_arguments)]
    pub fn prove(
        &self,
        id: &str,
        client: &[u8],
        authenticator: &[u8],
        signature: &[u8],
        purpose: &str,
        host: &str,
        rp: &str,
        now: u64,
    ) -> (Self, Result<(), String>) {
        let (mut next, challenge) = match self.take(client, purpose, now) {
            Ok(taken) => taken,
            Err(why) => return (self.clone(), Err(why)),
        };
        let Some(i) = next.keys.iter().position(|k| k.id == id) else {
            return (next, Err("this passkey isn't registered here".into()));
        };
        let checked = assert(
            &next.keys[i],
            client,
            authenticator,
            signature,
            &challenge,
            host,
            rp,
        );
        match checked {
            Ok(counter) => {
                next.keys[i].counter = counter;
                next.keys[i].used = Some(now);
                (next, Ok(()))
            }
            Err(why) => (next, Err(why)),
        }
    }

    /// Checks a `webauthn.get` answer from passkey `id` to a challenge minted by
    /// [`Self::mint_bound`] for `purpose`, against `bound` as it is now: the next keyring, whose
    /// challenge is used up even when the answer is refused, and the proof's evidence or why it
    /// was refused. A refused signature counts toward that passkey's lock, and no other's.
    #[allow(clippy::too_many_arguments)]
    pub fn prove_bound(
        &self,
        id: &str,
        client: &[u8],
        authenticator: &[u8],
        signature: &[u8],
        purpose: &str,
        bound: &Bound,
        host: &str,
        rp: &str,
        now: u64,
    ) -> (Self, Result<Evidence, String>) {
        let nonce = serde_json::from_slice::<serde_json::Value>(client)
            .ok()
            .and_then(|v| v["challenge"].as_str().map(str::to_string))
            .and_then(|answered| {
                self.challenges
                    .iter()
                    .find(|c| c.challenge == answered && c.purpose == purpose)
                    .and_then(|c| c.nonce.clone())
            });
        let (mut next, challenge) = match self.take(client, purpose, now) {
            Ok(taken) => taken,
            Err(why) => return (self.clone(), Err(why)),
        };
        let Some(nonce) = nonce else {
            return (
                next,
                Err("this challenge isn't bound; reload the page".into()),
            );
        };
        let raw = crate::push::b64url_decode(&nonce).unwrap_or_default();
        if bound_challenge(purpose, bound, &raw) != challenge {
            return (
                next,
                Err("what this passkey signed for has changed; reload the page".into()),
            );
        }
        let Some(i) = next.keys.iter().position(|k| k.id == id) else {
            return (next, Err("this passkey isn't registered here".into()));
        };
        next.keys[i].failures.retain(|at| *at + LOCK_WINDOW > now);
        if next.keys[i].locked(now) {
            return (
                next,
                Err("this passkey is locked for an hour; use another".into()),
            );
        }
        let checked = assert(
            &next.keys[i],
            client,
            authenticator,
            signature,
            &challenge,
            host,
            rp,
        );
        match checked {
            Ok(counter) => {
                next.keys[i].counter = counter;
                next.keys[i].used = Some(now);
                next.keys[i].failures.clear();
                let evidence = Evidence {
                    passkey: id.to_string(),
                    nonce,
                    client: crate::push::b64url(client),
                    authenticator: crate::push::b64url(authenticator),
                    signature: crate::push::b64url(signature),
                };
                (next, Ok(evidence))
            }
            Err(why) => {
                next.keys[i].failures.push(now);
                (next, Err(why))
            }
        }
    }

    /// Removes passkey `id` unless fewer than `min` would be left: the next keyring, and why it
    /// was refused.
    pub fn remove_keeping(&self, id: &str, min: usize) -> (Self, Result<(), String>) {
        if !self.holds(id) {
            return (
                self.clone(),
                Err("this passkey isn't registered here".into()),
            );
        }
        if self.keys.len() <= min {
            return (
                self.clone(),
                Err(format!("at least {min} passkeys stay; add another first")),
            );
        }
        let (next, _) = self.remove(id);
        (next, Ok(()))
    }

    /// Removes passkey `id`: the next keyring, and whether it was there.
    #[must_use]
    pub fn remove(&self, id: &str) -> (Self, bool) {
        let mut next = self.clone();
        next.keys.retain(|k| k.id != id);
        let removed = next.keys.len() != self.keys.len();
        (next, removed)
    }
}
