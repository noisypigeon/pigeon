//! The owner pages' presenters; see the ADR's 2026-11-18 amendment §2.

use super::{challenge, form};
use crate::present::checked;
use crate::view::View;
use serde_json::{Value, json};

fn fixture(has_inputs: bool) -> Value {
    let inputs = if has_inputs {
        json!([{ "key": "token", "label": "Token", "input": "secret", "required": true }])
    } else {
        json!([])
    };
    let mut r = json!({
        "page": "/approve/7?grant=ab",
        "operation": "grant.request",
        "target": "7",
        "head": "abc1234",
        "asked": {
            "title": "Approve the grant",
            "verb": "Approve",
            "body": "Lead sentence.",
            "fields": [
                { "label": "by", "value": { "kind": "text", "text": "owner" } },
                { "label": "for", "value": { "kind": "text", "text": "a token" } },
            ],
            "inputs": inputs,
            "refuse": true,
            "change": 7,
        },
    });
    if !has_inputs {
        r["asked"].as_object_mut().unwrap().remove("body");
    }
    r
}

#[test]
fn a_card_with_inputs_opens_the_form() {
    let c = challenge(&fixture(true)).unwrap();
    checked(View::Card, Some(c.clone())).unwrap();
    let a = c["actions"].as_array().unwrap();
    let tools: Vec<_> = a.iter().map(|x| x["tool"].as_str().unwrap()).collect();
    assert_eq!(
        tools,
        ["proof.form", "proof.submit", "change.status", "feed.read"]
    );
    assert_eq!(a[0]["label"], "Approve");
    assert_eq!(a[1]["label"], "Refuse");
    assert_eq!(a[2]["label"], "Open !7");
    assert_eq!(a[3]["label"], "Back to approvals");
    assert_eq!(c["body"], "Lead sentence.");
}

#[test]
fn a_card_without_inputs_submits() {
    let c = challenge(&fixture(false)).unwrap();
    checked(View::Card, Some(c.clone())).unwrap();
    let first = &c["actions"][0];
    assert_eq!(first["tool"], "proof.submit");
    assert_eq!(first["proof"], true);
    assert_eq!(first["arguments"]["decision"], "grant");
    assert_eq!(c["body"], "grant.request on 7");
    let fields = c["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields[3]["label"], "head");
}

#[test]
fn a_form_submits_with_proof() {
    let f = form(&fixture(true)).unwrap();
    checked(View::Form, Some(f.clone())).unwrap();
    assert_eq!(f["submit"]["tool"], "proof.submit");
    assert_eq!(f["submit"]["proof"], true);
    assert!(form(&fixture(false)).is_none());
}

#[test]
fn nothing_without_asked() {
    let r = json!({ "operation": "x", "target": "1" });
    assert!(challenge(&r).is_none());
    assert!(form(&r).is_none());
}
