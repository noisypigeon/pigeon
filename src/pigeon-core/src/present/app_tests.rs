//! The app family's presenters; see the ADR's R8 and #36 amendments.

use super::{NAMES, view};
use crate::wire::{Response, catalog};
use serde_json::{Value, json};

/// The view of `op` over `result`, checked against the operation's declared view.
fn present(op: &str, result: Value) -> Value {
    let r = Response {
        operation: op.to_string(),
        ok: true,
        result,
        violations: vec![],
    };
    let v = view(op, &r).unwrap_or_else(|| panic!("{op} has no view"));
    let declared = catalog()
        .iter()
        .find(|o| o.name == op)
        .expect("in the catalog")
        .view;
    crate::view::validate(declared, &v).unwrap_or_else(|p| panic!("{op}: {p:?}"));
    v
}

#[test]
fn device_show_and_push_are_cards() {
    let v = present("device.show", json!({ "device": "d1", "name": "Phone" }));
    assert_eq!(v["title"], "This device");
    assert_eq!(v["fields"][0]["value"]["text"], "d1");
    assert_eq!(v["fields"][1]["value"]["text"], "Phone");
    let v = present("device.push", json!({}));
    assert_eq!(v["title"], "Notifications on");
}

#[test]
fn instances_badge_the_current_one() {
    let v = present(
        "instances.list",
        json!({ "instances": [
            { "id": "a", "name": "A", "base_url": "https://a", "current": true, "status": "ready" },
            { "id": "b", "name": "B", "base_url": "https://b", "current": false, "status": "paused" },
        ] }),
    );
    assert_eq!(v["title"], "Instances");
    assert_eq!(v["items"][0]["badge"], "current");
    assert_eq!(v["items"][1]["badge"], "paused");
    assert_eq!(v["items"][1]["subtitle"], "https://b");
}

#[test]
fn members_then_invitations() {
    let v = present(
        "members.list",
        json!({
            "members": [{ "account": "m1", "role": "owner" }],
            "invitations": [{ "id": "i1", "email": "x@y.z", "status": "open" }],
        }),
    );
    assert_eq!(v["title"], "Members");
    assert_eq!(v["items"][0]["subtitle"], "owner");
    assert_eq!(v["items"][1]["title"], "x@y.z");
    assert_eq!(v["items"][1]["subtitle"], "invited");
    assert_eq!(v["items"][1]["badge"], "open");
}

#[test]
fn the_profile_has_its_sections_and_actions() {
    let v = present(
        "profile.show",
        json!({
            "profile": { "name": "Ann", "appearance": "dark" },
            "account": { "id": "acc" },
            "devices": [{ "name": "Phone", "current": true }, { "name": "Pad", "current": false }],
            "passkeys": [{ "name": "Key", "id": "k1" }],
        }),
    );
    assert_eq!(v["title"], "Profile");
    let sections = v["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["fields"][2]["value"]["text"], "acc");
    assert_eq!(sections[1]["heading"], "Devices");
    assert_eq!(sections[1]["fields"][0]["value"]["text"], "this device");
    assert_eq!(sections[1]["fields"][1]["value"]["text"], "linked");
    assert_eq!(sections[2]["fields"][0]["value"]["text"], "k1");
    let tools: Vec<_> = v["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["tool"].clone())
        .collect();
    assert_eq!(
        tools,
        [
            json!("profile.edit"),
            json!("avatar.edit"),
            json!("avatar.remove")
        ]
    );
    assert_eq!(v["actions"][2]["style"], "destructive");
}

#[test]
fn the_forms_name_their_inputs() {
    let v = present("profile.edit", json!({}));
    assert_eq!(v["fields"][1]["input"], "choice");
    assert_eq!(
        v["fields"][1]["options"],
        json!(["system", "light", "dark"])
    );
    assert_eq!(v["submit"]["tool"], "profile.update");
    let v = present("avatar.edit", json!({}));
    assert_eq!(v["fields"][0]["input"], "image");
    assert_eq!(v["submit"]["tool"], "avatar.put");
}

#[test]
fn saves_link_back_to_the_profile() {
    for (op, title) in [
        ("profile.update", "Profile saved"),
        ("avatar.put", "Photo saved"),
        ("avatar.remove", "Photo removed"),
    ] {
        let v = present(op, json!({}));
        assert_eq!(v["title"], title);
        assert_eq!(v["actions"][0]["tool"], "profile.show");
    }
    assert_eq!(present("avatar.show", json!({}))["title"], "Photo");
}

#[test]
fn the_proved_actions_present_saves() {
    for (op, title) in [
        ("device.unlink", "Device unlinked"),
        ("passkey.remove", "Passkey removed"),
    ] {
        assert_eq!(present(op, json!({}))["title"], title);
    }
}

#[test]
fn every_name_is_presented() {
    for op in NAMES {
        present(op, json!({}));
    }
}
