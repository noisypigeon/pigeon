//! pigeon's MCP server: its tools are the catalog's operations (control-plane ADR §10).

use backplane::Error;
use backplane::mcp::{self, Action, Link, PROTOCOL, PROTOCOL_URL, Session, code};
use backplane::wire::{Response, catalog};
use serde_json::{Value, json};

fn reply(message: &Value) -> Value {
    match mcp::parse(message, "0.0.0") {
        Action::Reply(v) => v,
        other => panic!("not a reply: {other:?}"),
    }
}

fn initialize(params: &Value) -> (Value, Session) {
    match mcp::parse(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params }),
        "0.0.0",
    ) {
        Action::Initialize { reply, session } => (reply, session),
        other => panic!("not an initialize: {other:?}"),
    }
}

#[test]
fn initialize_names_the_protocol_and_the_tools_capability() {
    let (r, session) = initialize(&json!({}));
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL);
    assert_eq!(session, Session::default());
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "pigeon");
}

#[test]
fn initialize_negotiates_url_elicitation_only_on_the_version_that_has_it() {
    let url = json!({ "elicitation": { "form": {}, "url": {} } });
    let (r, s) = initialize(&json!({ "protocolVersion": PROTOCOL_URL, "capabilities": url }));
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL_URL);
    assert!(s.url_elicitation);
    let (_, s) = initialize(&json!({ "protocolVersion": PROTOCOL_URL,
        "capabilities": { "elicitation": { "form": {} } } }));
    assert_eq!(s.protocol, PROTOCOL_URL);
    assert!(!s.url_elicitation, "form only");
    let (_, s) = initialize(
        &json!({ "protocolVersion": PROTOCOL_URL, "capabilities": { "elicitation": {} } }),
    );
    assert!(
        !s.url_elicitation,
        "an empty elicitation capability is form mode"
    );
    let (r, s) = initialize(&json!({ "protocolVersion": "2025-06-18", "capabilities": url }));
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL);
    assert!(!s.url_elicitation, "2025-06-18 has no URL mode");
}

#[test]
fn a_session_id_round_trips_under_its_key_and_nothing_else() {
    let key = b"0123456789abcdef0123456789abcdef";
    let s = Session {
        protocol: PROTOCOL_URL.into(),
        url_elicitation: true,
    };
    let id = mcp::session_id(&s, key);
    assert!(id.bytes().all(|b| b.is_ascii_graphic()), "{id}");
    assert_eq!(mcp::session(&id, key), Some(s.clone()));
    assert_eq!(mcp::session(&id, b"another key"), None);
    let forged = id.replacen(".u.", ".n.", 1);
    assert_eq!(mcp::session(&forged, key), None);
    let plain = mcp::session_id(&Session::default(), key);
    let upgraded = plain.replacen(".n.", ".u.", 1);
    assert_eq!(
        mcp::session(&upgraded, key),
        None,
        "a client can't grant itself URL mode"
    );
    let (payload, tag) = id.rsplit_once('.').expect("tag");
    let flipped = format!(
        "{payload}.{}{}",
        if tag.starts_with('0') { '1' } else { '0' },
        &tag[1..]
    );
    assert_eq!(mcp::session(&flipped, key), None);
    for bad in ["", "v1", "v1.x.u", "v1.x.u.zz", "v2.x.u.00"] {
        assert_eq!(mcp::session(bad, key), None, "{bad}");
    }
}

#[test]
fn a_link_is_a_url_elicitation_error_or_a_tool_result() {
    let link = Link {
        url: "https://host/approve/7?grant=abc".into(),
        message: "approve change 7".into(),
        id: "e1".into(),
    };
    let url = Session {
        protocol: PROTOCOL_URL.into(),
        url_elicitation: true,
    };
    let e = mcp::url_required(&json!(9), &link, &url);
    assert_eq!(e["id"], 9);
    assert!(e.get("result").is_none());
    assert_eq!(e["error"]["code"], -32042);
    assert_eq!(e["error"]["code"], code::URL_ELICITATION_REQUIRED);
    assert_eq!(
        e["error"]["data"]["elicitations"],
        json!([{ "mode": "url", "elicitationId": "e1", "url": link.url, "message": link.message }])
    );
    let r = mcp::url_required(&json!(9), &link, &Session::default());
    assert!(r.get("error").is_none());
    assert_eq!(r["result"]["isError"], false);
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains(&link.url)
    );
    assert_eq!(r["result"]["structuredContent"]["approval_url"], link.url);
}

#[test]
fn every_operation_is_a_tool_with_its_schema() {
    let r = reply(&json!({ "jsonrpc": "2.0", "id": "a", "method": "tools/list" }));
    let tools = r["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), catalog().len());
    let approve = tools
        .iter()
        .find(|t| t["name"] == "change_approve")
        .expect("change.approve");
    assert_eq!(approve["title"], "change.approve");
    assert_eq!(approve["inputSchema"]["required"], json!(["number"]));
    assert!(approve["inputSchema"]["properties"]["sha"].is_object());
    assert_eq!(approve["inputSchema"]["additionalProperties"], false);
    assert_eq!(approve["annotations"]["destructiveHint"], true);
    let status = tools
        .iter()
        .find(|t| t["name"] == "change_status")
        .expect("status");
    assert_eq!(status["annotations"]["readOnlyHint"], true);
    for t in tools {
        let name = t["name"].as_str().expect("name");
        assert!(
            name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{name}"
        );
    }
}

#[test]
fn a_call_becomes_a_request_and_its_outcome_a_tool_result() {
    let Action::Call { id, request } = mcp::parse(
        &json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                 "params": { "name": "change_status", "arguments": { "number": 3 } } }),
        "0",
    ) else {
        panic!("not a call");
    };
    assert_eq!(id, 7);
    assert_eq!(request.operation, "change.status");
    assert_eq!(request.args["number"], 3);
    let ok = mcp::answer(
        &id,
        Ok(Response {
            operation: "change.status".into(),
            ok: false,
            result: json!({ "state": "refused" }),
            violations: vec!["outside `scope`: x".into()],
        }),
    );
    assert_eq!(ok["result"]["isError"], true, "violations are a tool error");
    assert_eq!(
        ok["result"]["structuredContent"]["result"]["state"],
        "refused"
    );
    let err = mcp::answer(
        &id,
        Err(Error::Invalid(vec![
            "`change.status` needs argument `number`".into(),
        ])),
    );
    assert_eq!(err["result"]["isError"], true);
    assert!(
        err["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("needs argument")
    );
}

#[test]
fn bad_messages_get_json_rpc_errors_and_notifications_none() {
    let code_of = |m: Value| reply(&m)["error"]["code"].as_i64();
    assert_eq!(
        code_of(json!({ "id": 1, "method": "ping" })),
        Some(code::INVALID_REQUEST)
    );
    assert_eq!(
        code_of(json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list" })),
        Some(code::METHOD_NOT_FOUND)
    );
    assert_eq!(
        code_of(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "rm_rf" } })
        ),
        Some(code::INVALID_PARAMS)
    );
    assert_eq!(
        code_of(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": { "name": "change_list", "arguments": [1] } })),
        Some(code::INVALID_PARAMS)
    );
    assert!(matches!(
        mcp::parse(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            "0"
        ),
        Action::Accept
    ));
    assert_eq!(
        reply(&json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }))["result"],
        json!({})
    );
}
