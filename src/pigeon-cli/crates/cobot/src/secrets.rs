//! `secrets.config.yaml`: keys in clear, each value age/scrypt-encrypted; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §4.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use age::secrecy::SecretString;

use crate::Error;

/// The secrets file name inside a unit.
pub const FILE: &str = "secrets.config.yaml";

/// The environment variable holding the secrets passphrase.
pub const PASSPHRASE_VAR: &str = "COBOT_SECRETS_PASSPHRASE";

const PREFIX: &str = "ENC[cobot:v1:";

/// Whether `value` is an encrypted value.
#[must_use]
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX) && value.ends_with(']')
}

fn failed(path: &Path, message: impl Into<String>) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Reads the file's raw, still-encrypted values.
///
/// # Errors
///
/// [`Error::Io`] if it cannot be read or is not a map of strings.
pub fn read(path: &Path) -> Result<BTreeMap<String, String>, Error> {
    let text = fs::read_to_string(path).map_err(|e| failed(path, e.to_string()))?;
    serde_saphyr::from_str(&text).map_err(|e| failed(path, e.to_string()))
}

/// Encrypts one value.
///
/// # Errors
///
/// [`Error::Io`] if encryption fails.
pub fn encrypt(value: &str, passphrase: &str) -> Result<String, Error> {
    let encryptor =
        age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_string()));
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| failed(Path::new(FILE), e.to_string()))?;
    writer
        .write_all(value.as_bytes())
        .and_then(|()| writer.finish().map(|_| ()))
        .map_err(|e| failed(Path::new(FILE), e.to_string()))?;
    Ok(format!("{PREFIX}{}]", base64(&out)))
}

/// Decrypts one value.
///
/// # Errors
///
/// [`Error::Io`] if the value is malformed or the passphrase is wrong; the
/// message never includes the value.
pub fn decrypt(value: &str, passphrase: &str) -> Result<String, Error> {
    let bad = |why: &str| failed(Path::new(FILE), format!("cannot decrypt: {why}"));
    let body = value
        .strip_prefix(PREFIX)
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| bad("not ENC[cobot:v1:…]"))?;
    let bytes = unbase64(body).ok_or_else(|| bad("bad base64"))?;
    let decryptor = age::Decryptor::new(&bytes[..]).map_err(|_| bad("not an age file"))?;
    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| bad("wrong passphrase"))?;
    let mut out = String::new();
    reader
        .read_to_string(&mut out)
        .map_err(|_| bad("not UTF-8"))?;
    Ok(out)
}

/// Decrypts every value in `path`.
///
/// # Errors
///
/// [`Error::Io`] if the file cannot be read or any value cannot be decrypted.
pub fn decrypt_all(path: &Path, passphrase: &str) -> Result<BTreeMap<String, String>, Error> {
    read(path)?
        .into_iter()
        .map(|(k, v)| decrypt(&v, passphrase).map(|plain| (k, plain)))
        .collect()
}

/// Sets `key` to `value`, encrypted, creating the file if needed.
///
/// # Errors
///
/// [`Error::Io`] if the file cannot be read, encrypted or written.
pub fn set(path: &Path, key: &str, value: &str, passphrase: &str) -> Result<(), Error> {
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(failed(
            path,
            format!("key `{key}` must be lowercase letters, digits and underscores"),
        ));
    }
    let mut all = if path.exists() {
        read(path)?
    } else {
        BTreeMap::new()
    };
    all.insert(key.to_string(), encrypt(value, passphrase)?);
    let mut text = String::new();
    for (k, v) in &all {
        text.push_str(k);
        text.push_str(": ");
        text.push_str(v);
        text.push('\n');
    }
    fs::write(path, text).map_err(|e| failed(path, e.to_string()))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = u32::try_from(ALPHABET.iter().position(|a| *a == c)?).ok()?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{base64, decrypt, encrypt, is_encrypted, set, unbase64};

    #[test]
    fn keys_must_be_plain_names() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("secrets.config.yaml");
        for bad in ["", "Key", "a: b", "x\ny", "a-b"] {
            assert!(
                set(&path, bad, "v", "secrets-passphrase-0123").is_err(),
                "{bad:?}"
            );
        }
        assert!(!path.exists());
        set(&path, "account_id", "v", "secrets-passphrase-0123").expect("sets");
        assert!(path.exists());
    }

    #[test]
    fn base64_round_trips() {
        for s in [&b""[..], b"a", b"ab", b"abc", b"\x00\xff\x10binary"] {
            assert_eq!(unbase64(&base64(s)).expect("decodes"), s);
        }
        assert_eq!(base64(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn values_round_trip_and_wrong_passphrases_fail_quietly() {
        let enc = encrypt("s3cret-value", "secrets-passphrase-0123").expect("encrypts");
        assert!(is_encrypted(&enc));
        assert!(!enc.contains("s3cret-value"));
        assert_eq!(
            decrypt(&enc, "secrets-passphrase-0123").expect("decrypts"),
            "s3cret-value"
        );
        let err = decrypt(&enc, "another-passphrase-0123")
            .expect_err("wrong")
            .to_string();
        assert!(
            err.contains("wrong passphrase") && !err.contains("s3cret"),
            "{err}"
        );
        assert!(decrypt("plaintext", "x").is_err());
    }
}
