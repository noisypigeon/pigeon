use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use sha2::{Digest as _, Sha256};

use super::*;
use crate::core::member::{Role, Status};

const RP: &str = "platform.pigeon.dev";
const OWNER: &str = "acct_0123456789abcdefghjkmnpqrs";
const TOKEN: &str = "t0k3n";
const SPKI: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

fn point(k: &SigningKey) -> Vec<u8> {
    k.verifying_key()
        .to_encoded_point(false)
        .as_bytes()
        .to_vec()
}

fn invited() -> Members {
    let m = Members::default().seed(OWNER, 1);
    m.invite(TOKEN, "learner@example.com", Role::User, OWNER, 1)
        .unwrap()
        .0
}

fn request(challenge: &str, passkey: &SigningKey, device: &SigningKey) -> JoinRequest {
    let client = serde_json::json!({
        "type": "webauthn.create", "challenge": challenge, "origin": format!("https://{RP}")
    })
    .to_string();
    let mut auth = Sha256::digest(RP.as_bytes()).to_vec();
    auth.push(0x45);
    auth.extend([0, 0, 0, 0]);
    let mut spki = SPKI.to_vec();
    spki.extend(point(passkey));
    let sig: Signature = device.sign(join_message(TOKEN).as_bytes());
    JoinRequest {
        token: TOKEN.into(),
        name: "Learner".into(),
        device_name: "iPhone".into(),
        passkey_id: "cred-1".into(),
        client: b64url(client.as_bytes()),
        authenticator: b64url(&auth),
        spki: b64url(&spki),
        device_key: b64url(&point(device)),
        device_signature: b64url(&sig.to_bytes()),
    }
}

fn joined() -> (Joins, Members, Accounts, Invitation, SigningKey) {
    let device = SigningKey::from_slice(&[7; 32]).unwrap();
    let passkey = SigningKey::from_slice(&[9; 32]).unwrap();
    let members = invited();
    let (joins, challenge) = Joins::default()
        .challenge(&members, TOKEN, &[1; 32], 2)
        .unwrap();
    let (joins, r) = joins.join(
        members,
        Accounts::default(),
        &request(&challenge, &passkey, &device),
        RP,
        RP,
        &"ab".repeat(16),
        3,
    );
    let (members, accounts, inv) = r.unwrap();
    (joins, members, accounts, inv, device)
}

#[test]
fn an_invitee_enrols_with_a_native_passkey_and_their_device() {
    let (j, m, a, inv, device) = joined();
    assert_eq!(inv.status, InviteStatus::Enrolled);
    let code = inv.match_code.clone().unwrap();
    let passkey = SigningKey::from_slice(&[9; 32]).unwrap();
    assert_eq!(
        code,
        crate::core::member::match_code(TOKEN, &point(&passkey))
    );
    assert_eq!(a.accounts.len(), 1);
    assert_eq!(j.waiting[0].device.id, fingerprint(&point(&device)));
    assert_eq!(j.challenges, []);
    assert!(m.active(&a.accounts[0].id).is_none());
    assert_eq!(Joins::state(&m, &inv.id, 3), Some("enrolled"));
}

#[test]
fn admitting_links_the_device_as_a_user() {
    let (j, m, a, inv, _) = joined();
    let (j, m, d) = j.admit(m, Devices::default(), &inv.id, OWNER, 4).unwrap();
    let me = m.active(&a.accounts[0].id).unwrap();
    assert_eq!((me.role, me.status), (Role::User, Status::Active));
    assert_eq!(d.devices[0].account, a.accounts[0].id);
    assert_eq!(d.devices[0].linked, 4);
    assert_eq!(j.waiting, []);
    assert_eq!(j.passkeys[0].1.id, "cred-1");
    assert_eq!(Joins::state(&m, &inv.id, 4), Some("admitted"));
}

#[test]
fn only_an_administrator_admits() {
    let (j, m, a, inv, _) = joined();
    let invitee = a.accounts[0].id.clone();
    assert!(
        j.clone()
            .admit(m.clone(), Devices::default(), &inv.id, &invitee, 4)
            .is_err()
    );
    assert!(j.admit(m, Devices::default(), "nope", OWNER, 4).is_err());
}

#[test]
fn a_join_needs_its_challenge_its_passkey_and_its_device() {
    let device = SigningKey::from_slice(&[7; 32]).unwrap();
    let passkey = SigningKey::from_slice(&[9; 32]).unwrap();
    let m = invited();
    let (j, c) = Joins::default().challenge(&m, TOKEN, &[1; 32], 2).unwrap();
    let mut wrong_device = request(&c, &passkey, &device);
    wrong_device.device_key = b64url(&point(&passkey));
    let (j, r) = j.join(
        m.clone(),
        Accounts::default(),
        &wrong_device,
        RP,
        RP,
        &"ab".repeat(16),
        3,
    );
    assert!(r.is_err());
    // The challenge is used up even so.
    let (_, r) = j.join(
        m.clone(),
        Accounts::default(),
        &request(&c, &passkey, &device),
        RP,
        RP,
        &"ab".repeat(16),
        3,
    );
    assert_eq!(
        r.unwrap_err(),
        "this passkey answered no challenge for this link"
    );
    let (j, c) = Joins::default().challenge(&m, TOKEN, &[2; 32], 2).unwrap();
    let (_, r) = j.join(
        m.clone(),
        Accounts::default(),
        &request(&c, &passkey, &device),
        "evil.example",
        RP,
        &"ab".repeat(16),
        3,
    );
    assert!(r.is_err());
    assert!(
        Joins::default()
            .challenge(&m, "other", &[3; 32], 2)
            .is_err()
    );
    assert!(
        Joins::default()
            .challenge(&m, TOKEN, &[3; 32], 1 + crate::core::member::INVITE_TTL)
            .is_err()
    );
}

#[test]
fn purposes_name_what_they_prove() {
    assert_eq!(invite_purpose(" A@B.c "), "member:invite:a@b.c");
    assert_eq!(admit_purpose("x"), "member:admit:x");
}

#[test]
fn the_match_code_is_the_apps() {
    let mut point = vec![4u8];
    point.extend(1..=64u8);
    assert_eq!(crate::core::member::match_code(TOKEN, &point), "696816");
}
