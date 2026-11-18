//! The approval family's presenters; see the ADR's 2026-11-17 amendment on presenting.

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

fn link_of(v: &Value) -> Option<&Value> {
    v["fields"]
        .as_array()?
        .iter()
        .find(|f| f["label"] == "approve")
        .map(|f| &f["value"])
}

#[test]
fn money_is_dollars_and_cents() {
    assert_eq!(super::money(1_234_567), "$1.23");
    let v = present(
        "pool.status",
        json!({ "p": { "state": "on", "ledger": {
            "cycle": { "spent": 1_234_567, "limit": 5_000_000 }, "month": {}, "tasks": [] } } }),
    );
    assert_eq!(
        v["sections"][0]["fields"][1]["value"]["text"],
        "$1.23 of $5.00"
    );
}

#[test]
fn a_non_https_link_is_text() {
    let v = present(
        "grant.request",
        json!({ "pending": { "operation": "o", "link": "http://x" } }),
    );
    assert_eq!(link_of(&v).map(|l| l["kind"].clone()), Some(json!("text")));
}

#[test]
fn odd_shapes_still_validate() {
    for op in NAMES {
        present(op, json!("x"));
        let v = present(op, json!({}));
        assert!(!v["title"].as_str().unwrap_or("").trim().is_empty(), "{op}");
    }
}

#[test]
fn pool_pause_string_is_titled() {
    let v = present("pool.pause", json!("paused"));
    assert_eq!(v["title"], "Pool paused");
    assert_eq!(v["fields"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["fields"][0]["label"], "result");
}

#[test]
fn approvals_lists_lanes_then_waiting() {
    let v = present(
        "approval.list",
        json!({
            "lanes": [{ "id": 3, "reason": "r" }],
            "waiting": [
                { "path": "/a", "title": "A", "operation": "x", "expires": 5 },
                { "path": "/b", "title": "B", "operation": "y", "expires": 9 }
            ]
        }),
    );
    assert_eq!(v["title"], "Approvals");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["title"], "Fast lane #3 open");
    assert_eq!(items[0]["badge"], "open");
    assert_eq!(items[1]["title"], "B");
    assert_eq!(items[1]["needs"], "owner");
    assert_eq!(items[1]["open"]["tool"], "proof.challenge");
    assert_eq!(items[1]["open"]["arguments"]["page"], "/b");
}
