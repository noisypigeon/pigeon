//! The rendering contract's validation and text; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.

use super::*;
use serde_json::json;

fn problems(view: View, v: &Value) -> Vec<String> {
    validate(view, v).err().unwrap_or_default()
}

fn roundtrip<T: Copy + PartialEq + std::fmt::Debug>(
    all: &[T],
    name: fn(T) -> &'static str,
    parse: fn(&str) -> Option<T>,
) {
    let mut seen = Vec::new();
    for &v in all {
        assert_eq!(parse(name(v)), Some(v));
        assert!(!seen.contains(&name(v)), "duplicate {}", name(v));
        seen.push(name(v));
    }
    assert_eq!(parse("nope"), None);
}

#[test]
fn names_parse_back_and_are_unique() {
    roundtrip(&View::ALL, View::name, View::parse);
    roundtrip(&ValueKind::ALL, ValueKind::name, ValueKind::parse);
    roundtrip(&Input::ALL, Input::name, Input::parse);
    roundtrip(&Style::ALL, Style::name, Style::parse);
    roundtrip(&Needs::ALL, Needs::name, Needs::parse);
    roundtrip(&Mark::ALL, Mark::name, Mark::parse);
}

fn list_example() -> Value {
    json!({
        "view": "list",
        "title": "Changes",
        "items": [
            {
                "id": "1", "title": "feat x", "subtitle": "s", "badge": "open",
                "needs": "owner",
                "progress": {"stop": 2, "of": 5, "mark": "ok"},
                "open": {"label": "Open", "tool": "change.status", "arguments": {"n": 1},
                         "style": "plain", "proof": true}
            },
            {"id": "2", "title": "fix y", "needs": "none"}
        ],
        "next": "c2"
    })
}

fn card_example() -> Value {
    json!({
        "view": "card",
        "title": "Deploy",
        "body": "Rolling out",
        "fields": [
            {"label": "Note", "value": {"kind": "text", "text": "hello"}},
            {"label": "Cmd", "value": {"kind": "code", "text": "ls -l"}},
            {"label": "Head", "value": {"kind": "sha", "text": "0123456789abcdef0123"}},
            {"label": "At", "value": {"kind": "time", "at": 0}},
            {"label": "Took", "value": {"kind": "duration", "seconds": 3725}},
            {"label": "Runs", "value": {"kind": "count", "n": 4}}
        ],
        "actions": [
            {"label": "Merge", "tool": "change.merge", "arguments": {}, "style": "primary",
             "proof": false}
        ]
    })
}

fn detail_example() -> Value {
    json!({
        "view": "detail",
        "title": "Report",
        "sections": [
            {
                "heading": "Links",
                "fields": [
                    {"label": "Docs", "value": {"kind": "link", "text": "ADR",
                                                "href": "https://example.com/a"}},
                    {"label": "Done", "value": {"kind": "progress", "stop": 3, "of": 4,
                                                "mark": "stalled"}},
                    {"label": "Load", "value": {"kind": "series", "points": [
                        {"label": "a", "n": 1}, {"label": "b", "n": 2.5}]}}
                ],
                "text": "Notes here"
            },
            {"fields": []}
        ],
        "actions": [
            {"label": "Close", "tool": "change.close", "arguments": {"n": 1},
             "style": "destructive", "proof": false}
        ]
    })
}

fn submit() -> Value {
    json!({"label": "Save", "tool": "settings.save", "arguments": {}, "style": "primary",
           "proof": true})
}

fn form_example() -> Value {
    json!({
        "view": "form",
        "title": "Settings",
        "fields": [
            {"key": "name", "label": "Name", "input": "text", "required": true,
             "help": "Your name"},
            {"key": "kind", "label": "Kind", "input": "choice", "required": false,
             "options": ["a", "b"]},
            {"key": "on", "label": "On", "input": "toggle", "required": true},
            {"key": "n", "label": "N", "input": "number", "required": false},
            {"key": "pw", "label": "Pw", "input": "secret", "required": true}
        ],
        "submit": submit()
    })
}

#[test]
fn list_validates_and_renders() {
    let v = list_example();
    assert_eq!(validate(View::List, &v), Ok(()));
    assert_eq!(
        text(&v),
        "# Changes\n\
         - feat x · s [open] (needs owner) 2/5 ok → [Open] change.status {\"n\":1} (passkey)\n\
         - fix y\n\
         next: c2"
    );
}

#[test]
fn card_validates_and_renders() {
    let v = card_example();
    assert_eq!(validate(View::Card, &v), Ok(()));
    assert_eq!(
        text(&v),
        "# Deploy\n\
         Rolling out\n\
         Note: hello\n\
         Cmd: ls -l\n\
         Head: 0123456789ab\n\
         At: 1970-01-01 00:00 UTC\n\
         Took: 1h 2m\n\
         Runs: 4\n\
         - [Merge] change.merge {}"
    );
}

#[test]
fn detail_validates_and_renders() {
    let v = detail_example();
    assert_eq!(validate(View::Detail, &v), Ok(()));
    assert_eq!(
        text(&v),
        "# Report\n\
         ## Links\n\
         Docs: ADR <https://example.com/a>\n\
         Done: 3/4 stalled\n\
         Load: a 1, b 2.5\n\
         Notes here\n\
         - [Close] change.close {\"n\":1}"
    );
}

#[test]
fn form_validates_and_renders() {
    let v = form_example();
    assert_eq!(validate(View::Form, &v), Ok(()));
    assert_eq!(
        text(&v),
        "# Settings\n\
         - Name (name, text, required) — Your name\n\
         - Kind (kind, choice) [a, b]\n\
         - On (on, toggle, required)\n\
         - N (n, number)\n\
         - Pw (pw, secret, required)\n\
         submit: [Save] settings.save {} (passkey)"
    );
}

#[test]
fn proof_renders_passkey() {
    assert!(text(&list_example()).contains(" (passkey)"));
    let mut v = list_example();
    v["items"][0]["open"]["proof"] = json!(false);
    assert!(!text(&v).contains("(passkey)"));
}

fn list_with(items: &Value) -> Value {
    json!({"view": "list", "title": "T", "items": items})
}

fn card_with(fields: &Value) -> Value {
    json!({"view": "card", "title": "T", "fields": fields})
}

fn form_with(fields: &Value) -> Value {
    json!({"view": "form", "title": "T", "fields": fields, "submit": submit()})
}

#[test]
fn wrong_view() {
    let v = json!({"view": "card", "title": "T", "items": []});
    assert_eq!(problems(View::List, &v), ["$.view: expected list"]);
}

#[test]
fn missing_title() {
    let v = json!({"view": "list", "items": []});
    assert_eq!(problems(View::List, &v), ["$.title: missing"]);
}

#[test]
fn unknown_top_level_key() {
    let mut v = list_with(&json!([]));
    v["x"] = json!(1);
    assert_eq!(problems(View::List, &v), ["$.x: not in the contract"]);
}

#[test]
fn unknown_row_key() {
    let v = list_with(&json!([{"id": "a", "title": "t", "z": 1}]));
    assert_eq!(
        problems(View::List, &v),
        ["$.items[0].z: not in the contract"]
    );
}

#[test]
fn row_id_missing() {
    let v = list_with(&json!([{"id": "a", "title": "t"}, {"title": "t"}]));
    assert_eq!(problems(View::List, &v), ["$.items[1].id: missing"]);
}

#[test]
fn progress_stop_above_of() {
    let v = list_with(&json!([
        {"id": "a", "title": "t", "progress": {"stop": 5, "of": 3, "mark": "ok"}}
    ]));
    assert_eq!(
        problems(View::List, &v),
        ["$.items[0].progress.stop: above of"]
    );
}

#[test]
fn sha_not_hex() {
    let v = card_with(&json!([{"label": "H", "value": {"kind": "sha", "text": "XYZ1234"}}]));
    assert_eq!(
        problems(View::Card, &v),
        ["$.fields[0].value.text: expected 7 to 64 lowercase hex digits"]
    );
}

#[test]
fn link_not_https() {
    let v = card_with(&json!([
        {"label": "L", "value": {"kind": "link", "text": "x", "href": "http://example.com"}}
    ]));
    assert_eq!(
        problems(View::Card, &v),
        ["$.fields[0].value.href: expected a link starting https:// or pigeon://"]
    );
}

#[test]
fn choice_without_options() {
    let v = form_with(&json!([{"key": "k", "label": "L", "input": "choice", "required": true}]));
    assert_eq!(problems(View::Form, &v), ["$.fields[0].options: missing"]);
}

#[test]
fn text_with_options() {
    let v = form_with(&json!([
        {"key": "k", "label": "L", "input": "text", "required": true, "options": ["a"]}
    ]));
    assert_eq!(
        problems(View::Form, &v),
        ["$.fields[0].options: not in the contract"]
    );
}

#[test]
fn repeated_input_key() {
    let v = form_with(&json!([
        {"key": "k", "label": "A", "input": "text", "required": true},
        {"key": "k", "label": "B", "input": "text", "required": true}
    ]));
    assert_eq!(problems(View::Form, &v), ["$.fields[1].key: repeated"]);
}

fn card_tool(tool: &str) -> Value {
    json!({"view": "card", "title": "T", "actions": [
        {"label": "L", "tool": tool, "arguments": {}, "style": "plain", "proof": false}
    ]})
}

#[test]
fn tool_name_with_capital() {
    assert_eq!(
        problems(View::Card, &card_tool("Change.status")),
        ["$.actions[0].tool: not a tool name"]
    );
}

#[test]
fn tool_name_with_one_part() {
    assert_eq!(
        problems(View::Card, &card_tool("change")),
        ["$.actions[0].tool: not a tool name"]
    );
}

#[test]
fn detail_with_empty_sections() {
    let v = json!({"view": "detail", "title": "T", "sections": []});
    assert_eq!(
        problems(View::Detail, &v),
        ["$.sections: expected a non-empty array"]
    );
}

#[test]
fn all_problems_at_once() {
    let v = json!({"view": "card", "title": "", "x": 1, "items": []});
    assert_eq!(
        problems(View::List, &v),
        [
            "$.view: expected list",
            "$.title: expected a non-empty string",
            "$.x: not in the contract"
        ]
    );
}

#[test]
fn text_never_panics() {
    assert_eq!(text(&json!(null)), "# ");
    assert_eq!(text(&json!([])), "# ");
    assert_eq!(text(&json!({})), "# ");
}

#[test]
fn durations() {
    assert_eq!(render_duration(3725), "1h 2m");
    assert_eq!(render_duration(125), "2m 5s");
    assert_eq!(render_duration(9), "9s");
}

#[test]
fn time_at_zero() {
    assert_eq!(
        render_value(&json!({"kind": "time", "at": 0})),
        "1970-01-01 00:00 UTC"
    );
}
