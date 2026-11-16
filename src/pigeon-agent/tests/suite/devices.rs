//! The `Instance`'s `/devices/check` as the Worker drives it: stored state, a retried delivery and
//! a true replay; see the iOS ADR's 2026-11-16 amendment.

use backplane::core::app::devices::{Devices, canonical, link_message};
use backplane::push::b64url;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};

const NOW: u64 = 1_800_000_000;

fn sign(k: &SigningKey, data: &str) -> String {
    let s: Signature = k.sign(data.as_bytes());
    b64url(&s.to_bytes())
}

/// The object's storage, as JSON, with one linked device; and the device's ID.
fn stored(k: &SigningKey) -> (String, String) {
    let public = b64url(k.verifying_key().to_encoded_point(false).as_bytes());
    let d = Devices::default()
        .offer("iPhone", &public, "code", NOW)
        .expect("offered");
    let (d, r) = d.redeem("code", &public, &sign(k, &link_message("code")), NOW);
    let id = r.expect("linked").id;
    (serde_json::to_string(&d).expect("stored"), id)
}

/// One `/devices/check` call: reads storage, checks, writes storage back.
fn call(
    storage: &mut String,
    header: &str,
    path: &str,
    attempt: &str,
    now: u64,
) -> Result<String, String> {
    let d: Devices = serde_json::from_str(storage).expect("reads");
    let (d, r) = d.check_attempt(header, "GET", path, b"", attempt, now);
    *storage = serde_json::to_string(&d).expect("writes");
    r.map(|d| d.id)
}

#[test]
fn a_check_retried_after_its_answer_was_lost_is_not_a_replay() {
    let k = SigningKey::from_slice(&[9; 32]).expect("a key");
    let (mut storage, id) = stored(&k);
    let path = "/v0/app/home";
    let header = format!(
        "id={id};t={NOW};s={}",
        sign(&k, &canonical("GET", path, NOW, b""))
    );
    // `platform::call` resends the same body, attempt and all, after the object resets.
    let _lost = call(&mut storage, &header, path, "f00d", NOW);
    assert_eq!(
        call(&mut storage, &header, path, "f00d", NOW + 1),
        Ok(id.clone())
    );
    // A second delivery of the header, with the Worker's next attempt, is refused.
    let replay = call(&mut storage, &header, path, "beef", NOW + 2);
    assert!(replay.expect_err("replayed").contains("replayed"));
    // The app's re-signed retry is a new request.
    let t = NOW + 3;
    let fresh = format!(
        "id={id};t={t};s={}",
        sign(&k, &canonical("GET", path, t, b""))
    );
    assert_eq!(call(&mut storage, &fresh, path, "cafe", t), Ok(id));
}
