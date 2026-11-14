//! Passkeys: registering and asserting with ES256; see
//! `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §4.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

use backplane::passkeys::{
    ADD, Bound, Keyring, LOCK_AFTER, MAX_KEYS, Passkey, REGISTER, assert, bound_challenge,
    der_signature, grant_purpose, register, relying_party, remove_purpose,
};
use backplane::push::b64url;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest as _, Sha256};

const HOST: &str = "control.example.com";
const CHALLENGE: &str = "Y2hhbGxlbmdlLTAxMjM0NTY3ODk";

fn key() -> SigningKey {
    SigningKey::from_slice(&[7u8; 32]).expect("a scalar")
}

fn spki(k: &SigningKey) -> Vec<u8> {
    let mut out = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    out.extend_from_slice(k.verifying_key().to_encoded_point(false).as_bytes());
    out
}

fn client(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
    format!(
        r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{origin}","crossOrigin":false}}"#
    )
    .into_bytes()
}

fn authenticator(host: &str, flags: u8, counter: u32) -> Vec<u8> {
    let mut out = Sha256::digest(host.as_bytes()).to_vec();
    out.push(flags);
    out.extend_from_slice(&counter.to_be_bytes());
    out
}

fn der(sig: &Signature) -> Vec<u8> {
    let int = |b: &[u8]| {
        let b: Vec<u8> = b.iter().copied().skip_while(|x| *x == 0).collect();
        let mut v = if b.first().is_some_and(|x| x & 0x80 != 0) {
            vec![0]
        } else {
            vec![]
        };
        v.extend(b);
        let mut out = vec![0x02, u8::try_from(v.len()).expect("short")];
        out.extend(v);
        out
    };
    let bytes = sig.to_bytes();
    let mut body = int(&bytes[..32]);
    body.extend(int(&bytes[32..]));
    let mut out = vec![0x30, u8::try_from(body.len()).expect("short")];
    out.extend(body);
    out
}

fn sign(k: &SigningKey, authenticator: &[u8], client: &[u8]) -> Vec<u8> {
    let mut signed = authenticator.to_vec();
    signed.extend_from_slice(&Sha256::digest(client));
    let sig: Signature = k.sign(&signed);
    der(&sig)
}

fn registered(counter: u32) -> Passkey {
    let k = key();
    let (point, _) = register(
        &client("webauthn.create", CHALLENGE, &format!("https://{HOST}")),
        &authenticator(HOST, 0x05, 0),
        &spki(&k),
        CHALLENGE,
        HOST,
        HOST,
    )
    .expect("registers");
    Passkey {
        id: "cred".into(),
        key: b64url(&point),
        counter,
        name: "laptop".into(),
        created: 0,
        used: None,
        synced: None,
        failures: Vec::new(),
    }
}

#[test]
fn a_registered_passkey_proves_the_owner_once_per_challenge() {
    let k = key();
    let pk = registered(0);
    let origin = format!("https://{HOST}");
    let c = client("webauthn.get", CHALLENGE, &origin);
    let a = authenticator(HOST, 0x05, 0);
    assert_eq!(
        assert(&pk, &c, &a, &sign(&k, &a, &c), CHALLENGE, HOST, HOST),
        Ok(0)
    );
    let other = client("webauthn.get", "b3RoZXI", &origin);
    assert!(
        assert(
            &pk,
            &other,
            &a,
            &sign(&k, &a, &other),
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err(),
        "another link's"
    );
    let elsewhere = client("webauthn.get", CHALLENGE, "https://evil.example");
    assert!(
        assert(
            &pk,
            &elsewhere,
            &a,
            &sign(&k, &a, &elsewhere),
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err()
    );
    let rp = authenticator("evil.example", 0x05, 0);
    assert!(
        assert(&pk, &c, &rp, &sign(&k, &rp, &c), CHALLENGE, HOST, HOST).is_err(),
        "another RP"
    );
    let unverified = authenticator(HOST, 0x01, 0);
    assert!(
        assert(
            &pk,
            &c,
            &unverified,
            &sign(&k, &unverified, &c),
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err()
    );
    let forged = SigningKey::from_slice(&[9u8; 32]).expect("a scalar");
    assert!(
        assert(&pk, &c, &a, &sign(&forged, &a, &c), CHALLENGE, HOST, HOST).is_err(),
        "another key"
    );
    assert!(
        assert(&pk, &c, &a, b"\x30\x00", CHALLENGE, HOST, HOST).is_err(),
        "not a signature"
    );
}

#[test]
fn a_counter_that_goes_back_is_refused() {
    let k = key();
    let pk = registered(5);
    let c = client("webauthn.get", CHALLENGE, &format!("https://{HOST}"));
    let later = authenticator(HOST, 0x05, 6);
    assert_eq!(
        assert(
            &pk,
            &c,
            &later,
            &sign(&k, &later, &c),
            CHALLENGE,
            HOST,
            HOST
        ),
        Ok(6)
    );
    let same = authenticator(HOST, 0x05, 5);
    assert!(assert(&pk, &c, &same, &sign(&k, &same, &c), CHALLENGE, HOST, HOST).is_err());
}

#[test]
fn registration_takes_only_p256_from_this_site_with_verification() {
    let k = key();
    let origin = format!("https://{HOST}");
    let c = client("webauthn.create", CHALLENGE, &origin);
    let mut rsa = spki(&k);
    rsa[10] ^= 1;
    assert!(
        register(
            &c,
            &authenticator(HOST, 0x05, 0),
            &rsa,
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err()
    );
    assert!(
        register(
            &c,
            &authenticator(HOST, 0x01, 0),
            &spki(&k),
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err()
    );
    let get = client("webauthn.get", CHALLENGE, &origin);
    assert!(
        register(
            &get,
            &authenticator(HOST, 0x05, 0),
            &spki(&k),
            CHALLENGE,
            HOST,
            HOST
        )
        .is_err()
    );
    assert!(
        der_signature(&[0x30, 0x03, 0x02, 0x01, 0x01]).is_err(),
        "no s"
    );
}

#[test]
fn the_keyring_registers_and_proves_only_with_challenges_it_minted_for_that_purpose() {
    let k = key();
    let origin = format!("https://{HOST}");
    let mut ring = Keyring::default();
    let reg = kept!(ring.mint(REGISTER, &[1u8; 32], 100));
    let c = client("webauthn.create", &reg, &origin);
    let made = authenticator(HOST, 0x05, 0);
    assert!(
        kept!(ring.register("cred", "phone", &c, &made, &spki(&k), HOST, HOST, 101)).is_ok(),
        "registers"
    );
    assert!(
        kept!(ring.register("cred2", "x", &c, &made, &spki(&k), HOST, HOST, 102)).is_err(),
        "a challenge is used once"
    );
    let purpose = grant_purpose("token-for-change-7");
    let for_grant = kept!(ring.mint(&purpose, &[2u8; 32], 110));
    let got = client("webauthn.get", &for_grant, &origin);
    let a = authenticator(HOST, 0x05, 0);
    let sig = sign(&k, &a, &got);
    assert!(
        kept!(ring.prove(
            "cred",
            &got,
            &a,
            &sig,
            &grant_purpose("another"),
            HOST,
            HOST,
            111
        ))
        .is_err(),
        "another link's purpose"
    );
    let again = kept!(ring.mint(&purpose, &[3u8; 32], 112));
    let got = client("webauthn.get", &again, &origin);
    let sig = sign(&k, &a, &got);
    assert_eq!(
        kept!(ring.prove("cred", &got, &a, &sig, &purpose, HOST, HOST, 113)),
        Ok(())
    );
    assert_eq!(ring.keys[0].used, Some(113));
    assert!(
        kept!(ring.prove("cred", &got, &a, &sig, &purpose, HOST, HOST, 114)).is_err(),
        "a replay"
    );
    let late = kept!(ring.mint(&purpose, &[4u8; 32], 200));
    let got = client("webauthn.get", &late, &origin);
    let sig = sign(&k, &a, &got);
    assert!(
        kept!(ring.prove("cred", &got, &a, &sig, &purpose, HOST, HOST, 200 + 301)).is_err(),
        "a lapsed challenge"
    );
    assert!(kept!(ring.remove("cred")));
    assert!(!kept!(ring.remove("cred")));
}

#[test]
fn the_relying_party_is_the_configured_id_for_it_and_its_subdomains_only() {
    let rp = Some("platform.example.com");
    assert_eq!(
        relying_party("platform.example.com", rp),
        "platform.example.com"
    );
    assert_eq!(
        relying_party("blue.platform.example.com", rp),
        "platform.example.com"
    );
    assert_eq!(relying_party("x.workers.dev", rp), "x.workers.dev");
    assert_eq!(
        relying_party("evilplatform.example.com", rp),
        "evilplatform.example.com"
    );
    assert_eq!(
        relying_party("blue.platform.example.com", None),
        "blue.platform.example.com"
    );
    assert_eq!(relying_party("a.example.com", Some(" ")), "a.example.com");
}

#[test]
fn a_passkey_for_the_platform_registers_on_a_stack_and_proves_on_the_live_host() {
    let k = key();
    let rp = "platform.example.com";
    let (stack, live) = ("blue.platform.example.com", "platform.example.com");
    let mut ring = Keyring::default();
    let reg = kept!(ring.mint(REGISTER, &[1u8; 32], 100));
    let c = client("webauthn.create", &reg, &format!("https://{stack}"));
    let made = authenticator(rp, 0x05, 0);
    assert_eq!(
        kept!(ring.register("cred", "phone", &c, &made, &spki(&k), stack, rp, 101)),
        Ok(())
    );
    let purpose = grant_purpose("token-for-change-9");
    let a = authenticator(rp, 0x05, 0);
    let got = client(
        "webauthn.get",
        &kept!(ring.mint(&purpose, &[2u8; 32], 110)),
        &format!("https://{live}"),
    );
    let sig = sign(&k, &a, &got);
    assert_eq!(
        kept!(ring.prove("cred", &got, &a, &sig, &purpose, live, rp, 111)),
        Ok(())
    );
    let other = client(
        "webauthn.get",
        &kept!(ring.mint(&purpose, &[3u8; 32], 112)),
        &format!("https://{live}"),
    );
    let sig = sign(&k, &a, &other);
    assert!(
        kept!(ring.prove(
            "cred",
            &other,
            &a,
            &sig,
            &purpose,
            "x.workers.dev",
            "x.workers.dev",
            113
        ))
        .is_err(),
        "another site's page"
    );
}

/// A keyring holding passkeys named by `ids`, each registered with `k` and `flags`.
fn ring_of(k: &SigningKey, ids: &[&str], flags: u8) -> Keyring {
    let origin = format!("https://{HOST}");
    let mut ring = Keyring::default();
    for (n, id) in ids.iter().enumerate() {
        let at = 10 + u64::try_from(n).expect("few");
        let reg = kept!(ring.mint(REGISTER, &[9u8; 32], at));
        let c = client("webauthn.create", &reg, &origin);
        let made = authenticator(HOST, flags, 0);
        assert_eq!(
            kept!(ring.register(id, id, &c, &made, &spki(k), HOST, HOST, at)),
            Ok(())
        );
    }
    ring
}

fn approving(head: &str) -> Bound {
    Bound {
        operation: "change.approve".into(),
        target: "7".into(),
        head: head.into(),
    }
}

// See `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §2.
#[test]
fn a_bound_challenge_changes_with_each_thing_it_binds() {
    let b = approving("aaa");
    let base = bound_challenge("grant:k", &b, &[1u8; 32]);
    assert_eq!(base, bound_challenge("grant:k", &b, &[1u8; 32]));
    assert_ne!(base, bound_challenge("grant:other", &b, &[1u8; 32]));
    assert_ne!(
        base,
        bound_challenge("grant:k", &approving("bbb"), &[1u8; 32])
    );
    assert_ne!(base, bound_challenge("grant:k", &b, &[2u8; 32]));
    let merge = Bound {
        operation: "change.merge".into(),
        ..b.clone()
    };
    assert_ne!(base, bound_challenge("grant:k", &merge, &[1u8; 32]));
    let other = Bound {
        target: "8".into(),
        ..b
    };
    assert_ne!(base, bound_challenge("grant:k", &other, &[1u8; 32]));
}

#[test]
fn a_bound_proof_checks_only_for_the_head_it_was_minted_for_and_leaves_evidence() {
    let k = key();
    let origin = format!("https://{HOST}");
    let mut ring = ring_of(&k, &["cred"], 0x05);
    let purpose = grant_purpose("token");
    let a = authenticator(HOST, 0x05, 0);

    let minted = kept!(ring.mint_bound(&purpose, &approving("aaa"), &[3u8; 32], 100));
    let c = client("webauthn.get", &minted, &origin);
    let sig = sign(&k, &a, &c);
    let moved = approving("bbb");
    assert!(
        kept!(ring.prove_bound("cred", &c, &a, &sig, &purpose, &moved, HOST, HOST, 101)).is_err(),
        "the head moved"
    );
    assert!(
        kept!(ring.prove_bound(
            "cred",
            &c,
            &a,
            &sig,
            &purpose,
            &approving("aaa"),
            HOST,
            HOST,
            102
        ))
        .is_err(),
        "the challenge was used up"
    );

    let minted = kept!(ring.mint_bound(&purpose, &approving("aaa"), &[4u8; 32], 110));
    let c = client("webauthn.get", &minted, &origin);
    let sig = sign(&k, &a, &c);
    let evidence = kept!(ring.prove_bound(
        "cred",
        &c,
        &a,
        &sig,
        &purpose,
        &approving("aaa"),
        HOST,
        HOST,
        111
    ))
    .expect("proves");
    assert_eq!(evidence.passkey, "cred");
    assert_eq!(evidence.nonce, b64url(&[4u8; 32]));
    assert_eq!(evidence.client, b64url(&c));
    assert_eq!(ring.keys[0].used, Some(111));
    assert!(
        kept!(ring.prove_bound(
            "cred",
            &c,
            &a,
            &sig,
            &purpose,
            &approving("aaa"),
            HOST,
            HOST,
            112
        ))
        .is_err(),
        "a replay"
    );

    let plain = kept!(ring.mint(&purpose, &[5u8; 32], 120));
    let c = client("webauthn.get", &plain, &origin);
    assert!(!ring.bound(&c), "minted plain");
    let minted = kept!(ring.mint_bound(&purpose, &approving("aaa"), &[6u8; 32], 120));
    assert!(ring.bound(&client("webauthn.get", &minted, &origin)));
    let sig = sign(&k, &a, &c);
    assert!(
        kept!(ring.prove_bound(
            "cred",
            &c,
            &a,
            &sig,
            &purpose,
            &approving("aaa"),
            HOST,
            HOST,
            121
        ))
        .is_err(),
        "an unbound challenge"
    );
}

#[test]
fn five_refused_assertions_lock_that_passkey_and_no_other() {
    let k = key();
    let stranger = SigningKey::from_slice(&[8u8; 32]).expect("a scalar");
    let origin = format!("https://{HOST}");
    let mut ring = ring_of(&k, &["one", "two"], 0x05);
    let a = authenticator(HOST, 0x05, 0);
    let b = Bound::default();
    for n in 0..LOCK_AFTER {
        let at = 100 + u64::try_from(n).expect("few");
        let minted = kept!(ring.mint_bound(ADD, &b, &[u8::try_from(n).expect("few"); 32], at));
        let c = client("webauthn.get", &minted, &origin);
        let bad = sign(&stranger, &a, &c);
        assert!(kept!(ring.prove_bound("one", &c, &a, &bad, ADD, &b, HOST, HOST, at)).is_err());
    }
    assert!(ring.keys[0].locked(200));
    let prove = |ring: &mut Keyring, id: &str, at: u64| {
        let (next, minted) = ring.mint_bound(ADD, &b, &[77u8; 32], at);
        let c = client("webauthn.get", &minted, &origin);
        let sig = sign(&k, &a, &c);
        let (next, out) = next.prove_bound(id, &c, &a, &sig, ADD, &b, HOST, HOST, at);
        *ring = next;
        out
    };
    assert!(prove(&mut ring, "one", 201).is_err(), "locked");
    assert!(
        prove(&mut ring, "two", 202).is_ok(),
        "the other still proves"
    );
    assert!(prove(&mut ring, "nobody", 203).is_err(), "unknown");
    assert!(
        ring.keys
            .iter()
            .all(|k| k.id != "two" || k.failures.is_empty())
    );
    assert!(
        prove(&mut ring, "one", 100 + 3600 + 10).is_ok(),
        "an hour on"
    );
    assert_eq!(ring.keys[0].failures, Vec::<u64>::new());
}

#[test]
fn a_keyring_records_synced_passkeys_holds_eight_and_keeps_a_minimum() {
    let k = key();
    let ids = ["a", "b", "c", "d", "e", "f", "g", "h"];
    assert_eq!(ids.len(), MAX_KEYS);
    let mut ring = ring_of(&k, &ids, 0x0d);
    assert_eq!(ring.keys[0].synced, Some(true));
    assert_eq!(ring_of(&k, &["x"], 0x05).keys[0].synced, Some(false));
    let origin = format!("https://{HOST}");
    let reg = kept!(ring.mint(REGISTER, &[9u8; 32], 50));
    let c = client("webauthn.create", &reg, &origin);
    let made = authenticator(HOST, 0x05, 0);
    assert!(
        kept!(ring.register("i", "ninth", &c, &made, &spki(&k), HOST, HOST, 51)).is_err(),
        "a ninth"
    );
    assert!(ring.holds("a") && !ring.holds("i"));
    assert_eq!(remove_purpose("a"), "passkey:remove:a");

    let mut two = ring_of(&k, &["a", "b"], 0x05);
    assert!(kept!(two.remove_keeping("a", 2)).is_err(), "below two");
    assert!(kept!(two.remove_keeping("z", 1)).is_err(), "not here");
    assert_eq!(kept!(two.remove_keeping("a", 1)), Ok(()));
    assert!(kept!(two.remove_keeping("b", 1)).is_err(), "the last");
}

#[test]
fn a_keyring_stored_before_these_fields_still_reads() {
    let old = r#"{"keys":[{"id":"c","key":"k","counter":0,"name":"n","created":1}],
        "challenges":[{"challenge":"x","purpose":"register","expires":9}]}"#;
    let ring: Keyring = serde_json::from_str(old).expect("reads");
    assert_eq!(ring.keys[0].synced, None);
    assert_eq!(ring.keys[0].failures, Vec::<u64>::new());
    assert_eq!(ring.challenges[0].nonce, None);
}
