//! Access tokens: claims, keys and who may run what (control-plane ADR §9).

use backplane::access::{self, Identity};
use backplane::wire::catalog;
use serde_json::{Value, json};

fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

fn jwt(header: &Value, claims: &Value) -> String {
    format!(
        "{}.{}.{}",
        b64(header.to_string().as_bytes()),
        b64(claims.to_string().as_bytes()),
        b64(b"signature")
    )
}

fn claims(extra: &Value) -> Value {
    let mut c = json!({
        "iss": "https://pigeon.cloudflareaccess.com",
        "aud": ["app-aud"],
        "exp": 2000,
        "nbf": 900,
    });
    for (k, v) in extra.as_object().expect("object") {
        c[k] = v.clone();
    }
    c
}

fn who(extra: &Value) -> Result<Identity, String> {
    let t = access::decode(&jwt(
        &json!({ "alg": "RS256", "kid": "k1" }),
        &claims(extra),
    ))?;
    access::identity(&t, "pigeon", "app-aud", 1000)
}

#[test]
fn base64url_round_trips() {
    for s in ["", "a", "ab", "abc", "abcd", "hello, access"] {
        assert_eq!(
            access::base64url(&b64(s.as_bytes())).expect("decodes"),
            s.as_bytes()
        );
    }
    assert!(access::base64url("a+b").is_err());
}

#[test]
fn a_person_and_a_service_token_are_told_apart() {
    assert_eq!(
        who(&json!({ "email": "owner@example.com" })),
        Ok(Identity {
            subject: "owner@example.com".into(),
            human: true
        })
    );
    assert_eq!(
        who(&json!({ "common_name": "agent.access" })),
        Ok(Identity {
            subject: "agent.access".into(),
            human: false
        })
    );
    assert!(who(&json!({})).is_err(), "no one");
}

#[test]
fn every_claim_is_checked() {
    for (extra, want) in [
        (
            json!({ "email": "o@x", "iss": "https://other.cloudflareaccess.com" }),
            "another Access team",
        ),
        (
            json!({ "email": "o@x", "aud": "other" }),
            "another Access application",
        ),
        (json!({ "email": "o@x", "exp": 1000 }), "expired"),
        (json!({ "email": "o@x", "nbf": 2000 }), "not valid yet"),
    ] {
        let err = who(&extra).unwrap_err();
        assert!(err.contains(want), "{want}: {err}");
    }
    assert!(
        who(&json!({ "email": "o@x", "aud": "app-aud" })).is_ok(),
        "a single aud"
    );
}

#[test]
fn the_key_is_chosen_by_kid_and_only_rs256_is_taken() {
    let keys = json!({ "keys": [ { "kid": "k0", "kty": "RSA" }, { "kid": "k1", "kty": "RSA", "n": "x" } ] });
    let t = access::decode(&jwt(
        &json!({ "alg": "RS256", "kid": "k1" }),
        &claims(&json!({})),
    ))
    .expect("decodes");
    assert_eq!(access::key(&t, &keys).expect("finds")["n"], "x");
    assert_eq!(t.signature, b"signature");
    let none = access::decode(&jwt(
        &json!({ "alg": "none", "kid": "k1" }),
        &claims(&json!({})),
    ))
    .expect("decodes");
    assert!(access::key(&none, &keys).unwrap_err().contains("not RS256"));
    let rotated = access::decode(&jwt(
        &json!({ "alg": "RS256", "kid": "k9" }),
        &claims(&json!({})),
    ))
    .expect("decodes");
    assert!(
        access::key(&rotated, &keys)
            .unwrap_err()
            .contains("rotated")
    );
    assert!(access::decode("a.b").is_err());
}

#[test]
fn only_a_person_applies() {
    let person = Identity {
        subject: "owner@example.com".into(),
        human: true,
    };
    let agent = Identity {
        subject: "agent.access".into(),
        human: false,
    };
    let op = |n: &str| catalog().iter().find(|o| o.name == n).expect(n);
    for n in [
        "change.authorize",
        "change.approve",
        "change.merge",
        "change.close",
        "infra.apply",
    ] {
        assert!(access::authorize(op(n), &agent).is_err(), "{n}");
        assert!(access::authorize(op(n), &person).is_ok(), "{n}");
    }
    for n in [
        "change.propose",
        "change.pushed",
        "change.status",
        "ci.rerun",
    ] {
        assert!(access::authorize(op(n), &agent).is_ok(), "{n}");
    }
}
