//! Web Push's VAPID signing and the device list; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §3.

use backplane::push::{Devices, MAX_DEVICES, Vapid, audience, b64url, b64url_decode};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};

/// A fixed private scalar, for the tests only.
const SECRET: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";

#[test]
fn base64url_round_trips_without_padding() {
    assert_eq!(b64url(b"f"), "Zg");
    assert_eq!(b64url(b"fo"), "Zm8");
    assert_eq!(b64url(b"foo"), "Zm9v");
    assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
    for n in 0..40u8 {
        let bytes: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
        assert_eq!(b64url_decode(&b64url(&bytes)).expect("decodes"), bytes);
    }
    assert_eq!(b64url_decode("Zm8=").expect("padded"), b"fo");
    assert!(b64url_decode("a+b").is_err());
}

#[test]
fn a_vapid_header_verifies_with_its_own_public_key() {
    let vapid = Vapid::from_secret(SECRET).expect("a P-256 scalar");
    let public = vapid.public_key();
    let key = b64url_decode(&public).expect("decodes");
    assert_eq!((key.len(), key[0]), (65, 4), "uncompressed point");
    let header = vapid
        .authorization(
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://pigeon.example",
            1000,
        )
        .expect("signs");
    let (t, k) = header
        .strip_prefix("vapid t=")
        .and_then(|r| r.split_once(", k="))
        .expect("vapid t=…, k=…");
    assert_eq!(k, public);
    let parts: Vec<&str> = t.split('.').collect();
    assert_eq!(parts.len(), 3);
    let claims: serde_json::Value =
        serde_json::from_slice(&b64url_decode(parts[1]).expect("claims")).expect("JSON");
    assert_eq!(claims["aud"], "https://fcm.googleapis.com");
    assert_eq!(claims["exp"], 1000 + backplane::push::TOKEN_TTL);
    assert_eq!(claims["sub"], "https://pigeon.example");
    let sig = Signature::from_slice(&b64url_decode(parts[2]).expect("sig")).expect("64 bytes");
    VerifyingKey::from_sec1_bytes(&key)
        .expect("key")
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
        .expect("the signature verifies");
    assert!(Vapid::from_secret("AAAA").is_err(), "too short");
}

#[test]
fn an_audience_is_the_endpoints_origin() {
    assert_eq!(
        audience("https://web.push.apple.com/QGuQyavXutnMH/xyz").expect("ok"),
        "https://web.push.apple.com"
    );
    assert!(audience("http://insecure.example/x").is_err());
    assert!(audience("https:///x").is_err());
}

#[test]
fn devices_are_kept_once_each_and_at_most_ten() {
    let mut d = Devices::default();
    d.add("https://p.example/1", "owner", 1).expect("adds");
    d.add("https://p.example/1", "owner", 2).expect("again");
    assert_eq!(d.0.len(), 1);
    assert_eq!(d.0[0].at, 2);
    for i in 0..12 {
        d.add(&format!("https://p.example/n{i}"), "owner", 10 + i)
            .expect("adds");
    }
    assert_eq!(d.0.len(), MAX_DEVICES);
    assert_eq!(
        d.0[0].endpoint, "https://p.example/n2",
        "the oldest go first"
    );
    d.remove("https://p.example/n2");
    assert_eq!(d.0.len(), MAX_DEVICES - 1);
    assert!(d.add("ftp://x", "owner", 1).is_err());
}
