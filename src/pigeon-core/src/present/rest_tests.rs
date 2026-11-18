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
fn lists() {
    let v = shown(
        "git.tokens",
        json!({"tokens": [{"id": "t1", "scope": "read", "state": "live"}]}),
    );
    assert_eq!(title(&v), "Git tokens");
    assert_eq!(v["items"][0]["badge"], "live");
    let v = shown(
        "bench.list",
        json!({"benches": [{"name": "a", "used": 3, "busy": true}, {"name": "b", "used": 0}]}),
    );
    assert_eq!(title(&v), "Benches");
    assert_eq!(v["items"][0]["subtitle"], "used 3");
    assert_eq!(v["items"][0]["badge"], "busy");
    assert!(v["items"][1].get("badge").is_none());
    let v = shown(
        "events.wait",
        json!({"events": [{"id": 5, "summary": "s", "kind": "k", "change": 7,
            "stage": {"needs": "owner"}}]}),
    );
    assert_eq!(title(&v), "Events");
    assert_eq!(v["items"][0]["id"], "5");
    assert_eq!(v["items"][0]["badge"], "!7");
    assert_eq!(v["items"][0]["needs"], "owner");
    let v = shown(
        "provider.tools",
        json!({"tools": [{"name": "x", "drift": true}]}),
    );
    assert_eq!(title(&v), "Provider tools");
    assert_eq!(v["items"][0]["badge"], "drift");
    let v = shown(
        "feed.read",
        json!({"items": [{"id": 1, "summary": "s", "type": "t", "change": 9, "needs": "agent"}]}),
    );
    assert_eq!(title(&v), "Feed");
    assert_eq!(v["items"][0]["open"]["tool"], "change.status");
    assert_eq!(v["items"][0]["needs"], "agent");
}

#[test]
fn details() {
    let v = shown(
        "instance.show",
        json!({"name": "n", "description": "d", "schema": "s", "repository": "r"}),
    );
    assert_eq!(title(&v), "n");
    let v = shown(
        "architecture.describe",
        json!({"nodes": [1, 2], "edges": [1], "groups": [], "deployed": "abcdef1"}),
    );
    assert_eq!(title(&v), "Architecture");
    assert_eq!(v["sections"][0]["fields"][0]["value"]["n"], 2);
    let v = shown(
        "policy.check",
        json!({"ran": 4, "skipped": ["a"], "violations": ["v1", "v2"]}),
    );
    assert_eq!(title(&v), "Policy check");
    assert_eq!(v["sections"][1]["text"], "v1\nv2");
    let v = shown("infra.validate", json!({"units": [{"code": "ok"}]}));
    assert_eq!(v["sections"][0]["fields"][0]["label"], "1");
    let v = shown(
        "ios.status",
        json!({"in_flight": {"current": {"build": 3, "sha": "abcdef1", "stage": "s"},
            "pending": "p"}, "releases": [{"build": 2, "state": "done"}]}),
    );
    assert_eq!(title(&v), "iOS releases");
    assert_eq!(v["sections"][0]["heading"], "In flight");
    assert_eq!(v["sections"][1]["fields"][0]["label"], "#2");
    let v = shown(
        "apple.check",
        json!({"checks": {"a": {"ok": true}, "b": {"ok": false, "detail": "d"}}}),
    );
    assert_eq!(v["sections"][0]["fields"][1]["value"]["text"], "failed · d");
    let v = shown(
        "digest.show",
        json!({"needs_owner": true, "lines": ["a", "b"]}),
    );
    assert_eq!(v["sections"][0]["text"], "a\nb");
    for op in ["bench.status", "bench.logs"] {
        let v = shown(
            op,
            json!({"run": "r1", "state": "done", "exit": 0, "seconds": 4, "sha": "abcdef1", "log": "l"}),
        );
        assert_eq!(title(&v), "Run r1");
        assert_eq!(v["sections"][0]["text"], "l");
    }
    let v = shown(
        "infra.plan",
        json!({"plan": "p1", "expires": 9,
            "units": [{"unit": "u", "changes": [{"address": "a.b", "action": "create"}]}]}),
    );
    assert_eq!(title(&v), "Plan p1");
    assert_eq!(v["sections"][1]["heading"], "u");
}

#[test]
fn cards() {
    for op in ["ci.rerun", "ci.cloudflare"] {
        let v = shown(
            op,
            json!({"change": {"number": 7, "title": "t", "state": "checked", "head": "abcdef1"}}),
        );
        assert_eq!(title(&v), "!7 t");
        let v = shown(
            op,
            json!({"pending": {"operation": "o", "number": 7, "link": "https://x/y"}}),
        );
        assert_eq!(title(&v), "Waiting for the owner");
    }
    let v = shown(
        "ci.window",
        json!({"offer": {"hours": [1, 2], "expires": 9}, "pending": {"link": "https://x"}}),
    );
    assert_eq!(title(&v), "CI window offered");
    let v = shown(
        "sites.build",
        json!({"sites": [{"unit": "u", "template": "t"}]}),
    );
    assert_eq!(title(&v), "Sites built");
    let v = shown(
        "sites.publish",
        json!({"sites": [{"unit": "u", "put": 1, "unchanged": 2, "deleted": 3}]}),
    );
    assert_eq!(
        v["fields"][0]["value"]["text"],
        "put 1, unchanged 2, deleted 3"
    );
    let v = shown(
        "ios.release",
        json!({"build": 5, "state": "queued", "sha": "abcdef1", "auto": true}),
    );
    assert_eq!(title(&v), "iOS build 5");
    let v = shown(
        "bench.run",
        json!({"run": "r", "state": "s", "sha": "abcdef1", "seconds": 2}),
    );
    assert_eq!(title(&v), "Run r");
    assert_eq!(
        title(&shown("bench.cancel", json!({"run": "r"}))),
        "Cancelling r"
    );
    assert_eq!(
        title(&shown("bench.reset", json!({"ok": true}))),
        "Bench reset"
    );
    assert_eq!(title(&shown("bench.warm", json!({"warm": 1}))), "Bench");
    for op in ["owner.ask", "owner.withdraw"] {
        let v = shown(
            op,
            json!({"item": {"id": 3, "title": "t", "detail": "d", "kind": "k", "change": 7,
                "options": ["a", "b"], "answer": "a", "by": "o"}}),
        );
        assert_eq!(title(&v), "#3 t");
        assert_eq!(v["fields"][2]["value"]["text"], "a, b");
    }
    let v = shown(
        "infra.apply",
        json!({"plan": "p", "units": [{"unit": "u", "changes": [1, 2]}]}),
    );
    assert_eq!(title(&v), "Applied p");
    assert_eq!(v["fields"][0]["value"]["text"], "2 changes");
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
    assert_eq!(NAMES.len(), 31);
    for op in NAMES {
        for result in [json!("x"), json!({})] {
            let v = shown(op, result);
            assert!(!title(&v).trim().is_empty(), "{op}");
        }
    }
}

#[test]
fn build_tools_are_presented() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let v = shown(
        "build.list",
        json!({"sha": sha, "files": [{"name": "pigeon", "size": 10}]}),
    );
    assert_eq!(title(&v), "Build 0123456789ab");
    assert_eq!(v["items"][0]["subtitle"], "10 bytes");
    let v = shown(
        "build.read",
        json!({"sha": sha, "name": "pigeon", "offset": 0, "length": 3, "size": 9,
               "next": 3, "data": "SECRETDATA"}),
    );
    assert_eq!(title(&v), "pigeon from 0");
    assert!(!v.to_string().contains("SECRETDATA"));
}
