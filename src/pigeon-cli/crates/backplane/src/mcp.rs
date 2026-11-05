//! pigeon's MCP server: JSON-RPC 2.0 over streamable HTTP, one JSON answer per request, whose
//! tools are the catalog's operations (control-plane ADR §10). Pure and shared by both builds:
//! [`parse`] says what a message asks, the caller runs any operation, and [`answer`] wraps its
//! outcome.

use serde_json::{Value, json};

use crate::Error;
use crate::wire::{Operation, Request, Response, catalog};

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2025-06-18";

/// The JSON-RPC error codes it uses.
pub mod code {
    /// Not JSON-RPC.
    pub const INVALID_REQUEST: i64 = -32600;
    /// An unknown method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Bad parameters, or an unknown tool.
    pub const INVALID_PARAMS: i64 = -32602;
}

/// What a message asks.
#[derive(Debug, Clone)]
pub enum Action {
    /// Answer with this, already complete.
    Reply(Value),
    /// A notification: answer `202 Accepted` with no body.
    Accept,
    /// Run this operation, then [`answer`] with its outcome.
    Call {
        /// The request's ID.
        id: Value,
        /// The operation.
        request: Request,
    },
}

/// An operation's tool name: dots become underscores, which every MCP client accepts.
#[must_use]
pub fn tool_name(op: &Operation) -> String {
    op.name.replace('.', "_")
}

fn operation_of(tool: &str) -> Option<&'static Operation> {
    catalog().iter().find(|o| tool_name(o) == tool)
}

fn result(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC error.
#[must_use]
pub fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Every tool, from the catalog.
#[must_use]
pub fn tools() -> Value {
    let tools: Vec<Value> = catalog()
        .iter()
        .map(|o| {
            json!({
                "name": tool_name(o),
                "title": o.name,
                "description": o.summary,
                "inputSchema": o.input_schema(),
                "annotations": {
                    "readOnlyHint": o.effect == crate::wire::Effect::Read,
                    "destructiveHint": o.effect == crate::wire::Effect::Apply,
                },
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// What `message`, one JSON-RPC message, asks.
#[must_use]
pub fn parse(message: &Value, version: &str) -> Action {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Action::Reply(error(
            &id,
            code::INVALID_REQUEST,
            "not a JSON-RPC 2.0 message",
        ));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response from the client: nothing to answer.
        return Action::Accept;
    };
    if message.get("id").is_none() {
        return Action::Accept;
    }
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => Action::Reply(result(
            &id,
            &json!({
                "protocolVersion": PROTOCOL,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "pigeon", "version": version },
                "instructions": "pigeon's control plane. Propose a change, push its branch, read its status; only the owner approves.",
            }),
        )),
        "ping" => Action::Reply(result(&id, &json!({}))),
        "tools/list" => Action::Reply(result(&id, &tools())),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Action::Reply(error(&id, code::INVALID_PARAMS, "`name` is required"));
            };
            let Some(op) = operation_of(name) else {
                return Action::Reply(error(
                    &id,
                    code::INVALID_PARAMS,
                    &format!("no tool `{name}`"),
                ));
            };
            let args = match params.get("arguments") {
                None | Some(Value::Null) => serde_json::Map::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => {
                    return Action::Reply(error(
                        &id,
                        code::INVALID_PARAMS,
                        "`arguments` must be an object",
                    ));
                }
            };
            Action::Call {
                id,
                request: Request {
                    operation: op.name.to_string(),
                    args,
                },
            }
        }
        other => Action::Reply(error(
            &id,
            code::METHOD_NOT_FOUND,
            &format!("no method `{other}`"),
        )),
    }
}

/// The answer to a [`Action::Call`]: the operation's response as the tool's result, with
/// violations and errors as a tool error the model can read.
#[must_use]
pub fn answer(id: &Value, outcome: Result<Response, Error>) -> Value {
    match outcome {
        Ok(r) => {
            let text = serde_json::to_string_pretty(&r).unwrap_or_default();
            result(
                id,
                &json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": r,
                    "isError": !r.ok,
                }),
            )
        }
        Err(e) => result(
            id,
            &json!({
                "content": [{ "type": "text", "text": e.to_string() }],
                "isError": true,
            }),
        ),
    }
}
