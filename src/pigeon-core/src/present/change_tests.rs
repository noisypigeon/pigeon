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

fn change() -> Value {
    json!({
        "number": 7, "title": "feat: x", "state": "checked", "head": "abc1234def",
        "plan": "the plan",
        "jobs": [{"name": "fmt", "status": "passed", "attempts": 2}],
    })
}

#[test]
fn lists() {
    let v = shown(
        "change.list",
        json!({"changes": [{"number": 3, "title": "t", "state": "open", "model": "opus",
            "stage": {"stage": "checking", "needs": "agent"}}]}),
    );
    assert_eq!(title(&v), "Changes");
    assert_eq!(v["items"][0]["title"], "!3 t");
    assert_eq!(v["items"][0]["subtitle"], "checking");
    assert_eq!(v["items"][0]["needs"], "agent");
    let v = shown(
        "change.evictable",
        json!({"candidates": [{"number": 4, "fields": {"state": "open", "kind": "idle"}}]}),
    );
    assert_eq!(title(&v), "Eviction candidates");
    assert_eq!(v["items"][0]["badge"], "idle");
    let v = shown(
        "brief.list",
        json!({"briefs": [{"id": "b1", "title": "t", "adr": "a.md", "step": "1", "model": "sonnet"}]}),
    );
    assert_eq!(v["items"][0]["subtitle"], "a.md step 1");
    let tasks = json!({"tasks": [
        {"id": 1, "kind": "brief", "change": 0, "note": "n"},
        {"id": 2, "kind": "resume", "change": 5, "claimed_by": "s"},
        {"id": 3, "kind": "resume", "change": 5, "done": true},
    ]});
    for op in ["task.list", "task.wait"] {
        let v = shown(op, tasks.clone());
        assert_eq!(title(&v), "Tasks");
        assert_eq!(v["items"][0]["title"], "#1 brief on !0");
        assert_eq!(v["items"][0]["needs"], "agent");
        assert_eq!(v["items"][1]["badge"], "claimed");
        assert_eq!(v["items"][2]["badge"], "done");
    }
}

#[test]
fn details() {
    let v = shown(
        "change.scope",
        json!({"jobs": [{"name": "fmt", "argv": ["cargo", "fmt"]}]}),
    );
    assert_eq!(title(&v), "Checks");
    assert_eq!(v["sections"][0]["fields"][0]["value"]["text"], "cargo fmt");
    let v = shown(
        "session.status",
        json!({"session": "https://x", "model": "sonnet", "role": "executor",
            "events": {"listening": true},
            "changes": [{"number": 7, "title": "t", "stage": {"stage": "s", "needs": "agent"}}],
            "tasks": [{"id": 1, "kind": "brief", "note": "n"}]}),
    );
    assert_eq!(title(&v), "https://x");
    let v = shown(
        "brief.show",
        json!({"id": "b1", "revisions": [{"draft": {"title": "T", "adr": "a.md", "step": "1",
            "model": "sonnet", "base": "abc1234", "decisions": ["d1", "d2"],
            "steps": [{"do": "x", "done_when": "y"}], "commit": {"title": "c", "plan": "p"}}}]}),
    );
    assert_eq!(title(&v), "b1 T");
    assert_eq!(v["sections"][2]["text"], "d1\n\nd2");
    let v = shown(
        "change.status",
        json!({"change": change(), "branch": "change/7-x", "model": "sonnet", "position": 1,
            "approval": "waiting",
            "stage": {"stage": "checking", "needs": "agent", "next": "n", "waits_on": 3, "since": 9}}),
    );
    assert_eq!(title(&v), "!7 feat: x");
    assert_eq!(v["sections"][0]["heading"], "Stage");
    assert_eq!(v["sections"][2]["fields"][0]["value"]["text"], "passed ×2");
    assert_eq!(v["sections"][3]["text"], "the plan");
}

#[test]
fn change_cards() {
    let c = json!({"change": change()});
    for op in [
        "change.authorize",
        "change.pushed",
        "change.close",
        "change.approve",
        "change.merge",
    ] {
        let v = shown(op, c.clone());
        assert_eq!(title(&v), "!7 feat: x", "{op}");
        assert_eq!(v["body"], "state: checked");
    }
    let v = shown("change.merge", json!({"message": "m", "queued": 2}));
    assert_eq!(title(&v), "Queued to merge");
    assert_eq!(title(&shown("change.yield", change())), "!7 feat: x");
    let v = shown(
        "change.propose",
        json!({"number": 8, "branch": "b", "change": change(), "overlaps": [{}]}),
    );
    assert_eq!(title(&v), "Proposed !8");
    assert_eq!(
        title(&shown("change.pin", json!({"pinned": [1, 2]}))),
        "Pinned"
    );
    assert_eq!(
        title(&shown(
            "change.front",
            json!({"fronted": 3, "message": "m"})
        )),
        "Fronted !3"
    );
    assert_eq!(
        title(&shown("change.note", json!({"noted": 3}))),
        "Noted on !3"
    );
    assert_eq!(
        title(&shown("change.handoff", json!({"offered": 3}))),
        "Offered !3"
    );
}

#[test]
fn brief_and_session_cards() {
    let revs = json!([{"draft": {"title": "T"}}]);
    let v = shown(
        "brief.create",
        json!({"id": "b2", "revisions": revs, "route": {"route": "r", "why": "w"}}),
    );
    assert_eq!(title(&v), "b2 r1");
    assert_eq!(
        title(&shown(
            "brief.amend",
            json!({"id": "b2", "revisions": revs})
        )),
        "b2 r1"
    );
    assert_eq!(
        title(&shown(
            "brief.review",
            json!({"change": 4, "verdict": "ok", "brief": "b2"})
        )),
        "Reviewed !4"
    );
    assert_eq!(
        title(&shown(
            "brief.escalate",
            json!({"change": 4, "escalated": "q", "brief": "b2"})
        )),
        "Escalated !4"
    );
    assert_eq!(
        title(&shown(
            "session.start",
            json!({"label": "l", "model": "m", "role": "r", "expires": 9})
        )),
        "Session started"
    );
    let v = shown(
        "session.end",
        json!({"label": "l", "ended": "yes", "clean": false,
            "holds": [{"number": 2, "title": "t", "remains": {"resume": "go"}}]}),
    );
    assert_eq!(v["fields"][2]["value"]["text"], "go");
    assert_eq!(
        title(&shown(
            "session.archived",
            json!({"outcome": "archived", "why": "w", "by": "p", "at": 3})
        )),
        "Session archived"
    );
    for op in ["task.claim", "task.done"] {
        let v = shown(
            op,
            json!({"id": 9, "kind": "brief", "change": 1, "note": "n", "done": true, "closed": 1}),
        );
        assert_eq!(title(&v), "Task #9");
    }
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
    assert_eq!(NAMES.len(), 29);
    for op in NAMES {
        for result in [json!("x"), json!({})] {
            let v = shown(op, result);
            assert!(!title(&v).trim().is_empty(), "{op}");
        }
    }
}
