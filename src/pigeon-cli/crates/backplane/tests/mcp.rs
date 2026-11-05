//! pigeon's MCP server: its tools are the catalog's operations (control-plane ADR §10).

use backplane::Error;
use backplane::mcp::{self, Action, PROTOCOL, code};
use backplane::wire::{Response, catalog};
use serde_json::{Value, json};

fn reply(message: &Value) -> Value {
    match mcp::parse(message, "0.0.0") {
        Action::Reply(v) => v,
        other => panic!("not a reply: {other:?}"),
    }
}

#[test]
fn initialize_names_the_protocol_and_the_tools_capability() {
    let r = reply(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL);
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "pigeon");
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
    assert_eq!(approve["inputSchema"]["required"], json!(["number", "sha"]));
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
