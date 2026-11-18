//! The owner pages' presenters: a grant's `asked` as a card or a form; see
//! .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md's 2026-11-18 amendment §2.

use super::{action, card, code, field, sha};
use serde_json::{Value, json};

#[cfg(test)]
#[path = "proof_tests.rs"]
mod tests;

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn grant(r: &Value, asked: &Value) -> Value {
    action(
        str_of(asked, "verb"),
        "proof.submit",
        json!({ "page": r["page"], "operation": r["operation"], "decision": "grant" }),
        "primary",
        true,
    )
}

/// The card for a grant's `asked`; `None` without one.
#[must_use]
pub fn challenge(r: &Value) -> Option<Value> {
    let asked = r.get("asked").filter(|a| a.is_object())?;
    let operation = str_of(r, "operation");
    let body = match str_of(asked, "body") {
        "" => format!("{operation} on {}", str_of(r, "target")),
        b => b.to_string(),
    };
    let mut fields: Vec<Value> = asked["fields"].as_array().cloned().unwrap_or_default();
    fields.push(field("operation", code(operation)));
    let head = str_of(r, "head");
    if !head.is_empty() {
        fields.push(field("head", sha(head)));
    }
    let has_inputs = asked["inputs"].as_array().is_some_and(|i| !i.is_empty());
    let mut actions = vec![if has_inputs {
        action(
            str_of(asked, "verb"),
            "proof.form",
            json!({ "page": r["page"] }),
            "primary",
            false,
        )
    } else {
        grant(r, asked)
    }];
    if asked["refuse"] == json!(true) {
        actions.push(action(
            "Refuse",
            "proof.submit",
            json!({ "page": r["page"], "operation": r["operation"], "decision": "refuse" }),
            "destructive",
            true,
        ));
    }
    if let Some(n) = asked["change"].as_i64() {
        actions.push(action(
            &format!("Open !{n}"),
            "change.status",
            json!({ "number": n }),
            "plain",
            false,
        ));
    }
    actions.push(action(
        "Back to approvals",
        "feed.read",
        json!({ "needs": "owner", "open": "1" }),
        "plain",
        false,
    ));
    Some(card(str_of(asked, "title"), Some(&body), fields, actions))
}

/// The form for a grant's `asked` inputs; `None` without any.
#[must_use]
pub fn form(r: &Value) -> Option<Value> {
    let asked = r.get("asked").filter(|a| a.is_object())?;
    let inputs = asked["inputs"].as_array().filter(|i| !i.is_empty())?;
    Some(json!({
        "view": "form",
        "title": asked["title"],
        "fields": inputs,
        "submit": grant(r, asked),
    }))
}
