//! Accounts and profiles: what each check accepts and refuses.

use super::*;
use crate::core::app::devices::{Device, Devices};

const NOW: u64 = 1_800_000_000;
const HEX: &str = "00112233445566778899aabbccddeeff";

fn owner() -> (Accounts, Account) {
    Accounts::default().ensure_owner(HEX, NOW)
}

fn device(id: &str, account: &str) -> Device {
    Device {
        id: id.into(),
        name: "iPhone".into(),
        key: String::new(),
        linked: NOW,
        seen: NOW,
        push: None,
        account: account.into(),
    }
}

#[test]
fn ensure_owner_creates_one_person_account_and_profile() {
    let (a, acct) = owner();
    assert_eq!(a.accounts.len(), 1);
    assert_eq!(acct.kind, Kind::Person);
    assert_eq!(acct.created, NOW);
    let p = a.profile(&acct.id).unwrap();
    assert_eq!(p.name, DEFAULT_NAME);
    assert_eq!(p.appearance, Appearance::System);
    assert_eq!(p.avatar, None);
    assert_eq!(p.updated, NOW);
}

#[test]
fn ensure_owner_is_idempotent() {
    let (a, first) = owner();
    let (b, second) = a
        .clone()
        .ensure_owner("ffffffffffffffffffffffffffffffff", NOW + 5);
    assert_eq!(first, second);
    assert_eq!(a, b);
}

#[test]
fn account_id_is_acct_and_26_crockford_chars() {
    let id = account_id(HEX);
    let rest = id.strip_prefix(PREFIX).unwrap();
    assert_eq!(rest.len(), 26);
    assert!(rest.chars().all(|c| CROCKFORD.contains(c)));
}

#[test]
fn account_id_never_looks_like_32_hex() {
    let id = account_id(HEX);
    assert!(!(id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())));
    assert!(id.starts_with("acct_"));
}

#[test]
fn stamp_fills_only_empty_accounts() {
    let devices = Devices {
        devices: vec![device("a", ""), device("b", "acct_other")],
        ..Devices::default()
    };
    let d = stamp(devices, "acct_me");
    assert_eq!(d.devices[0].account, "acct_me");
    assert_eq!(d.devices[1].account, "acct_other");
}

#[test]
fn clean_name_refuses_empty_and_control_only() {
    assert!(clean_name("").is_err());
    assert!(clean_name("  ").is_err());
    assert!(clean_name("\u{7}\n").is_err());
}

#[test]
fn clean_name_keeps_64_chars() {
    let long = "x".repeat(100);
    assert_eq!(clean_name(&long).unwrap().chars().count(), 64);
    assert_eq!(clean_name(" a\u{7}b ").unwrap(), "ab");
}

#[test]
fn parse_appearance_refuses_unknown() {
    assert_eq!(parse_appearance("dark"), Ok(Appearance::Dark));
    assert_eq!(parse_appearance("light"), Ok(Appearance::Light));
    assert_eq!(parse_appearance("system"), Ok(Appearance::System));
    assert!(parse_appearance("sepia").is_err());
}

#[test]
fn update_sets_name_and_appearance() {
    let (a, acct) = owner();
    let (a, p) = a
        .update(&acct.id, Some(" Ada "), Some("dark"), NOW + 1)
        .unwrap();
    assert_eq!(p.name, "Ada");
    assert_eq!(p.appearance, Appearance::Dark);
    assert_eq!(p.updated, NOW + 1);
    assert_eq!(a.profile(&acct.id), Some(&p));
}

#[test]
fn update_refuses_unknown_account() {
    let (a, _) = owner();
    assert_eq!(
        a.update("acct_nope", Some("x"), None, NOW).unwrap_err(),
        "no such profile"
    );
}

#[test]
fn check_jpeg_refuses_non_jpeg() {
    assert!(check_jpeg(&[0x89, b'P', b'N', b'G']).is_err());
    assert!(check_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0]).is_ok());
}

#[test]
fn check_jpeg_refuses_over_64_kib() {
    let mut big = vec![0xFF, 0xD8, 0xFF];
    big.resize(AVATAR_MAX + 1, 0);
    assert!(check_jpeg(&big).is_err());
    big.truncate(AVATAR_MAX);
    assert!(check_jpeg(&big).is_ok());
}

#[test]
fn set_avatar_records_sha256_hex() {
    let (a, acct) = owner();
    let jpeg = [0xFF, 0xD8, 0xFF, 0x00];
    let (_, p) = a.set_avatar(&acct.id, &jpeg, NOW + 2).unwrap();
    let sum = p.avatar.unwrap();
    assert_eq!(sum.len(), 64);
    assert_eq!(sum, hex(&Sha256::digest(jpeg)));
}

#[test]
fn clear_avatar_removes_it() {
    let (a, acct) = owner();
    let (a, _) = a
        .set_avatar(&acct.id, &[0xFF, 0xD8, 0xFF, 0x00], NOW)
        .unwrap();
    let (_, p) = a.clear_avatar(&acct.id, NOW + 3).unwrap();
    assert_eq!(p.avatar, None);
    assert_eq!(p.updated, NOW + 3);
}

#[test]
fn may_remove_passkey_refuses_at_minimum() {
    assert!(may_remove_passkey(0).is_err());
    assert!(may_remove_passkey(1).is_err());
    assert!(may_remove_passkey(2).is_ok());
}

#[test]
fn purposes_are_spelled_as_the_adr_says() {
    assert_eq!(unlink_purpose("d1"), "unlink:d1");
    assert_eq!(passkey_remove_purpose("p1"), "passkey:remove:p1");
    assert_eq!(avatar_key("acct_x"), "avatar:acct_x");
}
