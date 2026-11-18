//! The rest family's presenters; see the ADR's 2026-11-17 amendment on presenting.

use super::{NAMES, view};
use crate::wire::{Response, catalog};
use serde_json::{Value, json};

/// The view of `op` over `result`, checked against the operation's declared view.
fn shown(op: &str, result: Value) -> Value {
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
        .unwrap_or_else(|| panic!("{op} is not in the catalog"))
        .view;
    crate::view::validate(declared, &v).unwrap_or_else(|p| panic!("{op}: {p:?}: {v}"));
    v
}

fn title(v: &Value) -> &str {
    v["title"].as_str().unwrap_or("")
}

#[test]
fn architecture_as_text_is_a_detail() {
    let v = shown("architecture.describe", json!("graph text"));
    assert_eq!(title(&v), "Architecture");
    assert_eq!(v["sections"][0]["text"], "graph text");
}

#[test]
fn provider_pending_is_a_card() {
    let v = shown("provider.call", json!({"pending": "waiting"}));
    assert_eq!(title(&v), "Pending");
    assert_eq!(v["body"], "waiting");
    let v = shown("provider.call", json!({"result": {"a": 1}}));
    assert_eq!(title(&v), "Provider call");
    let v = shown("provider.call", json!({"result": [1, 2]}));
    assert_eq!(v["fields"][0]["label"], "result");
}

#[test]
fn odd_shapes_still_validate() {
    for op in NAMES {
        for result in [json!("x"), json!({})] {
            let v = shown(op, result);
            assert!(!title(&v).trim().is_empty(), "{op}");
        }
    }
}

#[test]
fn the_web_push_tools_are_presented() {
    let v = shown("push.key", json!({ "key": "BPk" }));
    assert_eq!(title(&v), "Notifications");
    let v = shown("push.subscribe", json!({}));
    assert_eq!(title(&v), "This browser is notified");
}
