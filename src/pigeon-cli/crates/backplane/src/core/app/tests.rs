//! Device links and signed requests: what each accepts, and what a refusal keeps.

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};

use super::devices::{
    Devices, IDLE, LINK_TTL, SKEW, canonical, clean_name, fingerprint, key_point, link_message,
    parse_header,
};
use crate::push::b64url;

const NOW: u64 = 1_800_000_000;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("a key")
}

fn public(k: &SigningKey) -> String {
    b64url(k.verifying_key().to_encoded_point(false).as_bytes())
}

fn sign(k: &SigningKey, data: &str) -> String {
    let s: Signature = k.sign(data.as_bytes());
    b64url(&s.to_bytes())
}

fn linked(k: &SigningKey) -> (Devices, String) {
    let d = Devices::default()
        .offer("Willow's iPhone", &public(k), "code-1", NOW)
        .expect("offered");
    let (d, r) = d.redeem("code-1", &public(k), &sign(k, &link_message("code-1")), NOW);
    (d, r.expect("linked").id)
}

fn header(k: &SigningKey, id: &str, method: &str, path: &str, at: u64, body: &[u8]) -> String {
    format!(
        "id={id};t={at};s={}",
        sign(k, &canonical(method, path, at, body))
    )
}

#[test]
fn a_proved_link_is_redeemed_once_by_the_key_it_was_proved_for() {
    let k = key(7);
    let (d, id) = linked(&k);
    let point = key_point(&public(&k)).expect("point");
    assert_eq!(id, fingerprint(&point));
    assert_eq!(d.devices.len(), 1);
    assert_eq!(d.devices[0].name, "Willow's iPhone");
    assert!(d.offers.is_empty(), "the offer is used up");
    let (_, again) = d.redeem(
        "code-1",
        &public(&k),
        &sign(&k, &link_message("code-1")),
        NOW,
    );
    assert!(again.is_err(), "a code links once");
}

#[test]
fn a_link_is_refused_for_another_key_a_bad_signature_or_after_it_lapses() {
    let (k, other) = (key(7), key(8));
    let offered = || {
        Devices::default()
            .offer("phone", &public(&k), "c", NOW)
            .expect("offered")
    };
    let cases: Vec<(&str, Devices, String, String, u64)> = vec![
        (
            "another key",
            offered(),
            public(&other),
            sign(&other, &link_message("c")),
            NOW,
        ),
        (
            "signed by another key",
            offered(),
            public(&k),
            sign(&other, &link_message("c")),
            NOW,
        ),
        (
            "signed over another code",
            offered(),
            public(&k),
            sign(&k, &link_message("d")),
            NOW,
        ),
        (
            "lapsed",
            offered(),
            public(&k),
            sign(&k, &link_message("c")),
            NOW + LINK_TTL,
        ),
    ];
    for (what, d, key, sig, at) in cases {
        let (after, r) = d.redeem("c", &key, &sig, at);
        assert!(r.is_err(), "{what}");
        assert!(after.devices.is_empty(), "{what}");
        assert!(after.offers.is_empty(), "{what}: the offer is used up");
    }
}

#[test]
fn a_signed_request_names_its_device_once() {
    let k = key(7);
    let (d, id) = linked(&k);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW + 5, b"");
    let (d, r) = d.check(&h, "GET", "/v0/app/home", b"", NOW + 5);
    assert_eq!(r.expect("checked").id, id);
    assert_eq!(d.devices[0].seen, NOW + 5);
    let (_, replay) = d.check(&h, "GET", "/v0/app/home", b"", NOW + 6);
    assert!(replay.expect_err("replayed").contains("replayed"));
}

/// What a request case names: what differs, its header, method, path, body and time.
type Case<'a> = (&'a str, String, &'a str, &'a str, &'a [u8], u64);

#[test]
fn a_request_is_refused_when_anything_it_signed_differs() {
    let (k, other) = (key(7), key(8));
    let (d, id) = linked(&k);
    let body = br#"{"hold":false}"#;
    let ok = |at| header(&k, &id, "POST", "/v0/app/approvals/g", at, body);
    let cases: Vec<Case> = vec![
        (
            "another path",
            ok(NOW),
            "POST",
            "/v0/app/approvals/h",
            body,
            NOW,
        ),
        (
            "another method",
            ok(NOW),
            "PUT",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "another body",
            ok(NOW),
            "POST",
            "/v0/app/approvals/g",
            br#"{"hold":true}"#,
            NOW,
        ),
        (
            "too old",
            ok(NOW - SKEW - 1),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "from the future",
            ok(NOW + SKEW + 1),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "another key",
            header(&other, &id, "POST", "/v0/app/approvals/g", NOW, body),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "an unknown device",
            header(&k, "nobody", "POST", "/v0/app/approvals/g", NOW, body),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "a malformed header",
            "id=x".into(),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
    ];
    for (what, h, method, path, b, at) in cases {
        let (after, r) = d.clone().check(&h, method, path, b, at);
        assert!(r.is_err(), "{what}");
        assert!(after.seen.is_empty(), "{what}: a refusal records nothing");
    }
}

#[test]
fn an_unlinked_or_idle_device_is_refused() {
    let k = key(7);
    let (d, id) = linked(&k);
    let gone = d.clone().unlink(&id);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW, b"");
    assert!(gone.check(&h, "GET", "/v0/app/home", b"", NOW).1.is_err());
    let late = NOW + IDLE + 1;
    let h = header(&k, &id, "GET", "/v0/app/home", late, b"");
    assert!(d.check(&h, "GET", "/v0/app/home", b"", late).1.is_err());
}

#[test]
fn der_signatures_and_spki_keys_are_accepted_too() {
    let k = key(9);
    let spki = {
        let mut v = vec![
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ];
        v.extend_from_slice(k.verifying_key().to_encoded_point(false).as_bytes());
        b64url(&v)
    };
    let d = Devices::default()
        .offer("phone", &spki, "c", NOW)
        .expect("offered");
    let s: Signature = k.sign(link_message("c").as_bytes());
    let (_, r) = d.redeem("c", &spki, &b64url(s.to_der().as_bytes()), NOW);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn names_and_headers_are_read_strictly() {
    assert_eq!(clean_name("  "), "iPhone");
    assert_eq!(clean_name("a\u{0}b"), "ab");
    assert_eq!(clean_name(&"x".repeat(100)).len(), 64);
    assert_eq!(
        parse_header("id=a; t=12; s=sig").expect("parsed"),
        ("a".into(), 12, "sig".into())
    );
    assert!(parse_header("id=a;s=sig").is_err());
    assert!(key_point("not a key").is_err());
}

mod approvals {
    use serde_json::json;

    use crate::approve::Waiting;
    use crate::core::app::approvals::{detail, find, id, list, proof_form};
    use crate::core::approval::grants::{Grant, key};
    use crate::core::change::Change;

    fn waiting(number: u32, path: &str, expires: u64) -> Waiting {
        Waiting {
            number,
            operation: "change.approve".into(),
            title: format!("change {number}"),
            path: path.into(),
            expires,
        }
    }

    #[test]
    fn an_approval_is_named_by_its_grants_key_and_never_shows_its_token() {
        let w = vec![
            waiting(7, "/approve/7?grant=tok-seven", 100),
            waiting(9, "/approve/9?grant=tok-nine", 200),
            waiting(3, "/platform/3?grant=tok-platform", 300),
        ];
        let l = list(&w);
        let items = l.as_array().expect("a list");
        assert_eq!(items.len(), 2, "platform grants stay on the pages");
        assert_eq!(items[0]["number"], 9, "newest expiry first");
        assert_eq!(items[0]["id"], key("tok-nine"));
        assert!(!l.to_string().contains("tok-"), "no token leaves");
        let (found, token) = find(&w, &key("tok-seven")).expect("found");
        assert_eq!((found.number, token), (7, "tok-seven"));
        assert!(find(&w, &key("tok-platform")).is_none());
        assert!(find(&w, "nothing").is_none());
        assert_eq!(id(&w[2]), None);
    }

    #[test]
    fn the_detail_shows_the_card_and_the_proof_needed() {
        let w = waiting(7, "/approve/7?grant=tok", 100);
        let mut c = Change::propose(7, "x", "feat: x", "the plan", vec!["a/**".into()], "agent");
        c.head = Some("a".repeat(40));
        let g = Grant {
            operation: "change.approve".into(),
            number: 7,
            head: c.head.clone(),
            expires: 100,
            tier: None,
            used: None,
        };
        let d = detail(&w, &c, &g);
        assert_eq!(d["title"], "feat: x");
        assert_eq!(d["head"], "a".repeat(40));
        assert_eq!(d["scope"][0], "a/**");
        assert_eq!(d["needs_proof"], true, "no tier is the code tier");
        assert_eq!(d["id"], key("tok"));
    }

    #[test]
    fn the_passkey_answer_is_passed_on_as_base64url_only() {
        let body = json!({ "passkey": {
            "id": "abc_-1", "client": "eyJ0&x=1", "authenticator": "QUJD", "signature": "MEU"
        } });
        assert_eq!(
            proof_form(&body),
            "pk_id=abc_-1&pk_client=eyJ0x1&pk_authenticator=QUJD&pk_signature=MEU"
        );
    }
}
