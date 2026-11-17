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
fn lists_and_details() {
    let v = present(
        "grant.list",
        json!({ "grants": { "a": { "by": "owner" } }, "ledger": { "grants": { "a": {}, "b": {} } } }),
    );
    assert_eq!(v["title"], "Grants");
    assert_eq!(v["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(v["items"][0]["subtitle"], "by owner");
    assert_eq!(v["items"][1]["badge"], "not granted");

    let v = present(
        "mandate.status",
        json!({ "mandates": [
            { "id": 1, "summary": "s", "state": "active", "used": 2, "budget": 0, "hours": 3 },
            { "id": 2, "summary": "t", "state": "paused", "used": 2, "budget": 3, "hours": 1, "expires": 9, "why": "w" },
        ] }),
    );
    assert_eq!(v["title"], "Mandates");
    assert_eq!(v["sections"][0]["heading"], "#1 s");
    assert_eq!(v["sections"][0]["fields"][1]["value"]["kind"], "count");
    assert_eq!(v["sections"][1]["fields"][1]["value"]["mark"], "stalled");

    let v = present(
        "platform.status",
        json!({ "runs": [{ "id": 4, "unit": "u", "stage": "x", "created": 5, "sha": "abcdef1" }] }),
    );
    assert_eq!(v["title"], "Platform runs");
    assert_eq!(v["sections"][0]["fields"][2]["value"]["kind"], "sha");

    let v = present(
        "pool.status",
        json!({
            "idle": { "state": "unused" },
            "main": { "state": { "state": "on" }, "ledger": {
                "cycle": { "spent": 1_234_567, "limit": 5_000_000 },
                "month": { "spent": 0, "limit": 0 },
                "tasks": [{}, { "closed": 1 }], "paused": "owner" } },
        }),
    );
    assert_eq!(v["title"], "Pools");
    assert_eq!(v["sections"].as_array().map(Vec::len), Some(2));

    let v = present(
        "platform.plan",
        json!({ "run": { "id": 7, "unit": "u", "stage": "planned", "created": 1, "changes": [{ "a": 1 }] },
                "pending": { "link": "https://x.dev/a" } }),
    );
    assert_eq!(v["title"], "Plan #7 u");
    assert_eq!(v["sections"][1]["fields"][0]["label"], "change 1");

    let deploy = json!({ "pipeline": { "deployed": "abcdef1", "target": "abcdef2",
            "history": [{ "sha": "abcdef1234567890", "outcome": "ok", "published": ["a", "b"] }] },
        "mirror": { "mirrored": "abcdef1", "failures": 1 }, "mirror_behind": "2h" });
    let v = present("deploy.status", deploy.clone());
    assert_eq!(v["sections"].as_array().map(Vec::len), Some(3));
    assert_eq!(v["sections"][1]["fields"][0]["value"]["text"], "ok · a, b");
    let mut with = deploy;
    with["macos"] =
        json!({ "providers": [{ "provider": "bitrise", "health": "up", "minutes": 4 }] });
    let v = present("deploy.status", with);
    assert_eq!(v["sections"][3]["fields"][0]["value"]["text"], "up, 4 min");
}

#[test]
fn cards() {
    let pending = json!({ "operation": "o", "number": 3, "link": "https://x.dev/a" });
    let cases = [
        (
            "approval.wait",
            json!({ "message": "m", "pending": pending }),
            "Waiting for the owner",
        ),
        (
            "grant.request",
            json!({ "pending": pending }),
            "Asked the owner",
        ),
        (
            "grant.rotate",
            json!({ "pending": pending }),
            "Asked the owner",
        ),
        (
            "grant.revoke",
            json!({ "pending": pending }),
            "Asked the owner",
        ),
        (
            "mandate.propose",
            json!({ "mandate": { "id": 2, "summary": "s" }, "pending": pending }),
            "Mandate #2 proposed",
        ),
        (
            "mandate.end",
            json!({ "id": 2, "state": "ended", "summary": "s", "used": 4 }),
            "Mandate #2 ended",
        ),
        (
            "platform.apply",
            json!({ "run": { "id": 5, "unit": "u", "stage": "s" }, "pending": pending }),
            "Apply #5 u",
        ),
        (
            "pool.pause",
            json!({ "pool": "p", "ok": true }),
            "Pool paused",
        ),
        (
            "pool.resume",
            json!({ "offer": { "pool": "p", "expires": 8 }, "pending": pending }),
            "Resume offered",
        ),
        (
            "deploy.retry",
            json!({ "pipeline": { "target": "abcdef1" } }),
            "Deploy retried",
        ),
    ];
    for (op, result, title) in cases {
        let v = present(op, result);
        assert_eq!(v["title"], title, "{op}");
    }
    let v = present(
        "approval.wait",
        json!({ "message": "m", "pending": pending }),
    );
    assert_eq!(link_of(&v).map(|l| l["kind"].clone()), Some(json!("link")));
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
