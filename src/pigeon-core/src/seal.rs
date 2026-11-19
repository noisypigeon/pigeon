//! The Apple release's grant, sealed to the `sign` step's own key: HPKE (RFC 9180, base mode,
//! DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305), so the control plane carries only
//! ciphertext; see `.claude/decisions/2026-11-18-security-users-levels-capability-grants.md`,
//! its E2 amendment.

use chacha20::cipher::{KeyIvInit as _, StreamCipher as _};
use hpke::{Deserializable as _, Kem as _, OpModeR, OpModeS, Serializable as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

type Kem = hpke::kem::X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type Aead = hpke::aead::ChaCha20Poly1305;

/// What every seal's `info` starts with.
pub const INFO: &str = "pigeon release.apple";

/// A sealed grant: what the control plane carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    /// The encapsulated key, base64url.
    pub enc: String,
    /// The ciphertext, base64url.
    pub ciphertext: String,
    /// The recipient key's fingerprint.
    pub fingerprint: String,
}

/// The `info` a grant is sealed under: [`INFO`], then the build it was redeemed for.
#[must_use]
pub fn info(pipeline: &str, build: u64, commit: &str) -> Vec<u8> {
    format!("{INFO}\n{pipeline}\n{build}\n{commit}").into_bytes()
}

/// A public key's fingerprint: the first 16 hex digits of its SHA-256.
#[must_use]
pub fn fingerprint(public: &[u8]) -> String {
    crate::approval::grants::hex(&Sha256::digest(public)[..8])
}

/// A key pair from 32 random bytes: the private key's bytes and the public key, base64url.
#[must_use]
pub fn keypair(seed: &[u8; 32]) -> (Vec<u8>, String) {
    let (private, public) = Kem::derive_keypair(seed);
    (
        private.to_bytes().to_vec(),
        crate::mirroring::base64url(&public.to_bytes()),
    )
}

/// A cryptographic stream from a 32-byte seed the caller drew, so this module draws no
/// randomness of its own.
struct Stream(chacha20::ChaCha20);

impl rand_core::RngCore for Stream {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        dest.fill(0);
        self.0.apply_keystream(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for Stream {}

/// Seals `plaintext` to `recipient` (an X25519 public key, base64url) under `info`, with the
/// sender's randomness drawn from `seed`.
///
/// # Errors
///
/// When `recipient` is not an X25519 public key.
pub fn seal(
    recipient: &str,
    info: &[u8],
    plaintext: &[u8],
    seed: &[u8; 32],
) -> Result<Sealed, String> {
    let public = crate::access::base64url(recipient)?;
    let key = <Kem as hpke::Kem>::PublicKey::from_bytes(&public)
        .map_err(|_| "`recipient` is not an X25519 public key".to_string())?;
    let mut rng = Stream(chacha20::ChaCha20::new(seed.into(), &[0; 12].into()));
    let (enc, ciphertext) = hpke::single_shot_seal::<Aead, Kdf, Kem, _>(
        &OpModeS::Base,
        &key,
        info,
        plaintext,
        &[],
        &mut rng,
    )
    .map_err(|e| format!("sealing failed: {e}"))?;
    Ok(Sealed {
        enc: crate::mirroring::base64url(&enc.to_bytes()),
        ciphertext: crate::mirroring::base64url(&ciphertext),
        fingerprint: fingerprint(&public),
    })
}

/// Opens `sealed` with `private`, the key pair's private bytes, under `info`.
///
/// # Errors
///
/// When the key, the `info` or the ciphertext don't match.
pub fn open(private: &[u8], info: &[u8], sealed: &Sealed) -> Result<Vec<u8>, String> {
    let key = <Kem as hpke::Kem>::PrivateKey::from_bytes(private)
        .map_err(|_| "not an X25519 private key".to_string())?;
    let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(&crate::access::base64url(&sealed.enc)?)
        .map_err(|_| "`enc` is not an encapsulated key".to_string())?;
    let ciphertext = crate::access::base64url(&sealed.ciphertext)?;
    hpke::single_shot_open::<Aead, Kdf, Kem>(&OpModeR::Base, &key, &enc, info, &ciphertext, &[])
        .map_err(|_| "the grant does not open with this key for this build".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_grant_opens_only_with_its_key_for_its_build() {
        let (private, public) = keypair(&[7; 32]);
        let (other, _) = keypair(&[8; 32]);
        let mine = info("pigeon-ios-build", 41, "abc");
        let sealed = seal(&public, &mine, b"grant", &[9; 32]).unwrap();
        assert!(!sealed.ciphertext.contains("grant"));
        assert_eq!(open(&private, &mine, &sealed), Ok(b"grant".to_vec()));
        assert!(open(&other, &mine, &sealed).is_err());
        assert!(open(&private, &info("pigeon-ios-build", 42, "abc"), &sealed).is_err());
    }

    #[test]
    fn a_recipient_that_is_no_key_is_refused_and_a_fingerprint_names_the_key() {
        assert!(seal("not-a-key", b"i", b"g", &[1; 32]).is_err());
        let (_, public) = keypair(&[7; 32]);
        let bytes = crate::access::base64url(&public).unwrap();
        assert_eq!(fingerprint(&bytes).len(), 16);
        assert_eq!(
            seal(&public, b"i", b"g", &[1; 32]).unwrap().fingerprint,
            fingerprint(&bytes)
        );
    }
}
