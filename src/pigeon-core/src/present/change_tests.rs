//! The change family's presenters; see the ADR's 2026-11-17 amendment on presenting.

use super::{NAMES, view};
use crate::wire::{Response, catalog};
use serde_json::{Value, json};

fn response(op: &str, result: Value) -> Response {
    Response {
        operation: op.to_string(),
        ok: true,
        result,
        violations: vec![],
    }
}

/// The view of `op`'s `result`, checked against the operation's declared view.
fn shown(op: &str, result: Value) -> Value {
    let v = view(op, &response(op, result)).unwrap_or_else(|| panic!("{op} has no view"));
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
fn the_ticket_is_never_shown() {
    let ticket = "SECRET-TICKET-VALUE.1.2.3";
    let v = shown(
        "session.start",
        json!({"label": "l", "model": "m", "role": "r", "expires": 9, "ticket": ticket}),
    );
    assert!(!v.to_string().contains(ticket));
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
