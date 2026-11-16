//! The MCP client's requests, response parsing and the tool schema hash.

use std::fmt;

use serde_json::{Value, json};

/// The MCP protocol version the client speaks.
pub const PROTOCOL: &str = crate::mcp::PROTOCOL_URL;

/// The `initialize` request.
#[must_use]
pub fn initialize(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "pigeon", "version": env!("CARGO_PKG_VERSION")}
        }
    })
}

/// The `notifications/initialized` notification.
#[must_use]
pub fn initialized() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// The `tools/list` request, from `cursor` when given.
#[must_use]
pub fn tools_list(id: u64, cursor: Option<&str>) -> Value {
    let params = cursor.map_or_else(|| json!({}), |c| json!({"cursor": c}));
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/list", "params": params})
}

/// The `tools/call` request.
#[must_use]
pub fn tools_call(id: u64, name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
}

/// The SHA-256 hex of a tool's `inputSchema`, keys sorted.
///
/// # Panics
/// Never: a `Value` always serializes.
#[must_use]
pub fn schema_hash(input_schema: &Value) -> String {
    common::hex::sha256_hex(
        serde_json::to_string(input_schema)
            .expect("a Value serializes")
            .as_bytes(),
    )
}

/// Why a call to a server failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The request did not complete.
    Transport(String),
    /// The server answered with a JSON-RPC error.
    Rpc {
        /// The error code.
        code: i64,
        /// The error message.
        message: String,
    },
    /// The tool reported an error.
    Tool(String),
    /// The response was not shaped as expected.
    Shape(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) => write!(f, "transport: {m}"),
            Self::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
            Self::Tool(m) => write!(f, "tool error: {m}"),
            Self::Shape(m) => write!(f, "unexpected response: {m}"),
        }
    }
}

fn sse_events(body: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data: Vec<&str> = Vec::new();
    for line in body.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !data.is_empty() {
                events.push(data.join("\n"));
                data.clear();
            }
        } else if let Some(d) = line.strip_prefix("data:") {
            data.push(d.strip_prefix(' ').unwrap_or(d));
        }
    }
    events
}

/// The `result` of the response with `id`, from a JSON or SSE body.
///
/// # Errors
/// Rpc for a JSON-RPC error, Shape when no usable message has the id.
pub fn response(body: &str, content_type: &str, id: u64) -> Result<Value, ClientError> {
    let message = if content_type.starts_with("text/event-stream") {
        sse_events(body)
            .iter()
            .filter_map(|e| serde_json::from_str::<Value>(e).ok())
            .find(|v| v["id"].as_u64() == Some(id))
    } else {
        serde_json::from_str::<Value>(body).ok()
    };
    let Some(message) = message else {
        return Err(ClientError::Shape(format!("no response with id {id}")));
    };
    if let Some(error) = message.get("error") {
        return Err(ClientError::Rpc {
            code: error["code"].as_i64().unwrap_or(0),
            message: error["message"].as_str().unwrap_or_default().to_string(),
        });
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| ClientError::Shape("a response with neither result nor error".into()))
}

/// A `tools/call` result's value: `structuredContent`, else `content`.
///
/// # Errors
/// Tool, with the text items joined, when `isError` is true.
pub fn tool_result(result: &Value) -> Result<Value, ClientError> {
    let content = result.get("content").cloned().unwrap_or(Value::Null);
    if result["isError"].as_bool() == Some(true) {
        let text = content
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["type"] == "text")
                    .filter_map(|i| i["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        return Err(ClientError::Tool(text));
    }
    Ok(result.get("structuredContent").cloned().unwrap_or(content))
}

/// One page of `tools/list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Each tool's name and schema hash, sorted by name.
    pub tools: Vec<(String, String)>,
    /// The cursor of the next page.
    pub next: Option<String>,
}

/// Reads a `tools/list` result.
///
/// # Errors
/// Shape when a tool lacks `name` or `inputSchema`.
pub fn listed(result: &Value) -> Result<Listed, ClientError> {
    let mut tools = Vec::new();
    for t in result["tools"].as_array().map_or(&[][..], Vec::as_slice) {
        match (t["name"].as_str(), t.get("inputSchema")) {
            (Some(n), Some(s)) => tools.push((n.to_string(), schema_hash(s))),
            _ => {
                return Err(ClientError::Shape(
                    "a listed tool lacks name or inputSchema".into(),
                ));
            }
        }
    }
    tools.sort();
    Ok(Listed {
        tools,
        next: result["nextCursor"].as_str().map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_have_their_shape() {
        assert_eq!(
            initialize(1),
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion": PROTOCOL, "capabilities": {},
                "clientInfo": {"name":"pigeon","version":env!("CARGO_PKG_VERSION")}}})
        );
        assert_eq!(
            initialized(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        );
        assert_eq!(
            tools_list(2, None),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
        );
        assert_eq!(
            tools_list(2, Some("c")),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"cursor":"c"}})
        );
        assert_eq!(
            tools_call(3, "t", &json!({"a": 1})),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"t","arguments":{"a":1}}})
        );
    }

    #[test]
    fn the_schema_hash_ignores_key_order() {
        let a = schema_hash(&json!({"b": 1, "a": 2}));
        let b = schema_hash(&serde_json::from_str::<Value>(r#"{"a":2,"b":1}"#).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, schema_hash(&json!({"a": 3})));
    }

    #[test]
    fn response_reads_json_and_sse() {
        let json_body = r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#;
        assert_eq!(
            response(json_body, "application/json", 7),
            Ok(json!({"ok": true}))
        );
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        assert_eq!(
            response(sse, "text/event-stream; charset=utf-8", 7),
            Ok(json!({"ok": true}))
        );
    }

    #[test]
    fn response_refuses_errors_and_missing_ids() {
        let err = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no"}}"#;
        assert_eq!(
            response(err, "application/json", 1),
            Err(ClientError::Rpc {
                code: -32601,
                message: "no".into()
            })
        );
        assert_eq!(
            response("", "text/event-stream", 5),
            Err(ClientError::Shape("no response with id 5".into()))
        );
        assert_eq!(
            response(r#"{"jsonrpc":"2.0","id":1}"#, "application/json", 1),
            Err(ClientError::Shape(
                "a response with neither result nor error".into()
            ))
        );
    }

    #[test]
    fn tool_result_reads_each_form() {
        let e = json!({"isError": true, "content": [
            {"type":"text","text":"a"},{"type":"image"},{"type":"text","text":"b"}]});
        assert_eq!(tool_result(&e), Err(ClientError::Tool("a\nb".into())));
        let s = json!({"structuredContent": {"x": 1}, "content": [{"type":"text","text":"x"}]});
        assert_eq!(tool_result(&s), Ok(json!({"x": 1})));
        let c = json!({"content": [{"type":"text","text":"x"}]});
        assert_eq!(tool_result(&c), Ok(json!([{"type":"text","text":"x"}])));
    }

    #[test]
    fn listed_sorts_pages_and_refuses_a_bad_tool() {
        let r = json!({"tools": [
            {"name": "b", "inputSchema": {"a": 1}},
            {"name": "a", "inputSchema": {}}], "nextCursor": "n"});
        let l = listed(&r).unwrap();
        assert_eq!(l.tools[0], ("a".to_string(), schema_hash(&json!({}))));
        assert_eq!(l.tools[1].0, "b");
        assert_eq!(l.next.as_deref(), Some("n"));
        assert_eq!(listed(&json!({"tools": []})).unwrap().next, None);
        assert_eq!(
            listed(&json!({"tools": [{"name": "a"}]})),
            Err(ClientError::Shape(
                "a listed tool lacks name or inputSchema".into()
            ))
        );
    }

    #[test]
    fn errors_display() {
        assert_eq!(ClientError::Tool("x".into()).to_string(), "tool error: x");
    }
}
