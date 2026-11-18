//! The app's device-door presenters; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md, its R8 amendment.

use super::{action, card, code, detail, field, list, row, section, text};
use serde_json::{Value, json};

/// The operation-name prefixes of this family; empty for the rest.
#[allow(dead_code)]
pub const PREFIXES: &[&str] = &["device.", "instances.", "members.", "profile.", "avatar."];

/// How many of this family's operations have no presenter yet; each brief that presents some lowers it.
#[allow(dead_code)]
pub const PENDING: usize = 0;

/// The operations this family presents.
pub const NAMES: &[&str] = &[
    "device.show",
    "device.push",
    "instances.list",
    "members.list",
    "profile.show",
    "profile.edit",
    "profile.update",
    "avatar.show",
    "avatar.edit",
    "avatar.put",
    "avatar.remove",
    "device.unlink",
    "passkey.remove",
];

/// The view of operation `op`'s result, when this family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<Value> {
    let v = &r.result;
    let out = match op {
        "device.show" => card(
            "This device",
            None,
            vec![
                field("device", code(s(v, "device"))),
                field("name", text(s(v, "name"))),
            ],
            vec![],
        ),
        "device.push" => card(
            "Notifications on",
            Some("This device gets the owner's pushes."),
            vec![],
            vec![],
        ),
        "instances.list" => list("Instances", instances(v), None),
        "members.list" => list("Members", members(v), None),
        "profile.show" => profile(v),
        "profile.edit" => json!({
            "view": "form",
            "title": "Profile",
            "fields": [
                { "key": "name", "label": "Name", "input": "text", "required": false },
                {
                    "key": "appearance",
                    "label": "Appearance",
                    "input": "choice",
                    "required": false,
                    "options": ["system", "light", "dark"],
                },
            ],
            "submit": action("Save", "profile.update", json!({}), "primary", false),
        }),
        "profile.update" => saved("Profile saved"),
        "avatar.show" => card("Photo", None, vec![], vec![]),
        "avatar.edit" => json!({
            "view": "form",
            "title": "Photo",
            "fields": [
                { "key": "jpeg", "label": "Photo", "input": "image", "required": true },
            ],
            "submit": action("Save photo", "avatar.put", json!({}), "primary", false),
        }),
        "avatar.put" => saved("Photo saved"),
        "avatar.remove" => saved("Photo removed"),
        "device.unlink" => saved("Device unlinked"),
        "passkey.remove" => saved("Passkey removed"),
        _ => return None,
    };
    Some(out)
}

/// A string at `k` in `v`; empty when missing.
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// The entries of the array at `k` in `v`; empty when missing.
fn entries<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map_or(&[], Vec::as_slice)
}

/// A card that names its success and links back to the profile.
fn saved(title: &str) -> Value {
    card(
        title,
        None,
        vec![],
        vec![action("Profile", "profile.show", json!({}), "plain", false)],
    )
}

fn instances(v: &Value) -> Vec<Value> {
    entries(v, "instances")
        .iter()
        .map(|i| {
            let badge = if i["current"].as_bool() == Some(true) {
                "current"
            } else {
                s(i, "status")
            };
            row(
                s(i, "id"),
                s(i, "name"),
                Some(s(i, "base_url")),
                Some(badge),
                None,
                None,
            )
        })
        .collect()
}

fn members(v: &Value) -> Vec<Value> {
    let mut rows: Vec<Value> = entries(v, "members")
        .iter()
        .map(|m| {
            row(
                s(m, "account"),
                s(m, "account"),
                Some(s(m, "role")),
                None,
                None,
                None,
            )
        })
        .collect();
    rows.extend(entries(v, "invitations").iter().map(|i| {
        row(
            s(i, "id"),
            s(i, "email"),
            Some("invited"),
            Some(s(i, "status")),
            None,
            None,
        )
    }));
    rows
}

fn profile(v: &Value) -> Value {
    let head = section(
        None,
        vec![
            field("name", text(s(&v["profile"], "name"))),
            field("appearance", text(s(&v["profile"], "appearance"))),
            field("account", code(s(&v["account"], "id"))),
        ],
        None,
    );
    let devices = entries(v, "devices")
        .iter()
        .map(|d| {
            let state = if d["current"].as_bool() == Some(true) {
                "this device"
            } else {
                "linked"
            };
            field(s(d, "name"), text(state))
        })
        .collect();
    let passkeys = entries(v, "passkeys")
        .iter()
        .map(|p| field(s(p, "name"), text(s(p, "id"))))
        .collect();
    detail(
        "Profile",
        vec![
            head,
            section(Some("Devices"), devices, None),
            section(Some("Passkeys"), passkeys, None),
        ],
        vec![
            action("Edit", "profile.edit", json!({}), "plain", false),
            action("Change photo", "avatar.edit", json!({}), "plain", false),
            action(
                "Remove photo",
                "avatar.remove",
                json!({}),
                "destructive",
                false,
            ),
        ],
    )
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
