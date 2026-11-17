//! pigeon's MCP server: JSON-RPC 2.0 over streamable HTTP, one JSON answer per request, whose
//! tools are the catalog's operations (control-plane ADR §10). Pure and shared by both builds:
//! [`parse`] says what a message asks, the caller runs any operation, and [`answer`] wraps its
//! outcome.

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::approval::grants::hex;
use crate::wire::Error;
use crate::wire::{Operation, Request, Response, catalog};

/// The protocol version this server speaks by default.
pub const PROTOCOL: &str = "2025-06-18";

/// The protocol version with URL-mode elicitation, spoken when a client offers it.
pub const PROTOCOL_URL: &str = "2025-11-25";

/// The prefix of the IDs of elicitations this server asks.
pub const ELICIT: &str = "elicit-";

/// The header carrying a session's ID.
pub const SESSION_HEADER: &str = "Mcp-Session-Id";

/// The JSON-RPC error codes it uses.
pub mod code {
    /// Not JSON-RPC.
    pub const INVALID_REQUEST: i64 = -32600;
    /// An unknown method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Bad parameters, or an unknown tool.
    pub const INVALID_PARAMS: i64 = -32602;
}

/// What a client declared at `initialize`, carried in its session ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The negotiated protocol version.
    pub protocol: String,
    /// Whether the client shows URL-mode elicitations.
    pub url_elicitation: bool,
    /// Whether the client shows form-mode elicitations.
    pub form_elicitation: bool,
    /// The door it was opened on: "access" or "device".
    pub door: String,
    /// The first 16 hex digits of the SHA-256 of its caller's subject; empty in an ID older than v3.
    pub caller: String,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL.into(),
            url_elicitation: false,
            form_elicitation: false,
            door: "access".into(),
            caller: String::new(),
        }
    }
}

fn mac(key: &[u8], payload: &str) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(payload.as_bytes());
    m
}

/// The first 16 hex digits of the SHA-256 of `subject`: how a session ID names its caller.
#[must_use]
pub fn caller_tag(subject: &str) -> String {
    let mut tag = hex(&Sha256::digest(subject.as_bytes()));
    tag.truncate(16);
    tag
}

/// `session`'s ID, signed with `key`: `v3.<protocol>.<caps>.<door>.<caller>.<hex HMAC-SHA256>`, where
/// `caps` holds `f` for form and `u` for URL elicitation, or is `n` for neither.
#[must_use]
pub fn session_id(session: &Session, key: &[u8]) -> String {
    let mut caps = String::new();
    if session.form_elicitation {
        caps.push('f');
    }
    if session.url_elicitation {
        caps.push('u');
    }
    if caps.is_empty() {
        caps.push('n');
    }
    let payload = format!(
        "v3.{}.{caps}.{}.{}",
        session.protocol, session.door, session.caller
    );
    let tag = hex(&mac(key, &payload).finalize().into_bytes());
    format!("{payload}.{tag}")
}

/// The session `id` names, if `key` signed it; `v1` IDs carry URL elicitation only, and `v1` and
/// `v2` IDs name no door or caller.
#[must_use]
pub fn session(id: &str, key: &[u8]) -> Option<Session> {
    let (payload, tag) = id.rsplit_once('.')?;
    let tag: Vec<u8> = (0..tag.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(tag.get(i..i + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    mac(key, payload).verify_slice(&tag).ok()?;
    let parts: Vec<&str> = payload.split('.').collect();
    match parts.as_slice() {
        ["v1", protocol, caps] => Some(Session {
            protocol: (*protocol).to_string(),
            url_elicitation: *caps == "u",
            ..Session::default()
        }),
        ["v2", protocol, caps] => Some(Session {
            protocol: (*protocol).to_string(),
            url_elicitation: caps.contains('u'),
            form_elicitation: caps.contains('f'),
            ..Session::default()
        }),
        ["v3", protocol, caps, door, caller]
            if matches!(*door, "access" | "device")
                && caller.len() == 16
                && caller
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            Some(Session {
                protocol: (*protocol).to_string(),
                url_elicitation: caps.contains('u'),
                form_elicitation: caps.contains('f'),
                door: (*door).to_string(),
                caller: (*caller).to_string(),
            })
        }
        _ => None,
    }
}

/// Whether a request may use session ID `id`: none, an older ID, or one opened on this door by this caller.
#[must_use]
pub fn admits(id: Option<&str>, key: &[u8], door: &str, subject: &str) -> bool {
    let Some(id) = id else {
        return true;
    };
    let Some(session) = session(id, key) else {
        return false;
    };
    session.caller.is_empty() || (session.door == door && session.caller == caller_tag(subject))
}

/// The session a client's `initialize` params ask for: `2025-11-25` when offered, else
/// `2025-06-18`. Form elicitation is an `elicitation` capability that is empty or names `form`;
/// URL elicitation names `url`, in `2025-11-25` only.
#[must_use]
pub fn negotiate(params: &Value) -> Session {
    let elicitation = &params["capabilities"]["elicitation"];
    let form = elicitation
        .as_object()
        .is_some_and(|o| o.is_empty() || o.get("form").is_some_and(Value::is_object));
    if params["protocolVersion"].as_str() == Some(PROTOCOL_URL) {
        Session {
            protocol: PROTOCOL_URL.into(),
            url_elicitation: elicitation["url"].is_object(),
            form_elicitation: form,
            ..Session::default()
        }
    } else {
        Session {
            form_elicitation: form,
            ..Session::default()
        }
    }
}

/// What a message asks.
#[derive(Debug, Clone)]
pub enum Action {
    /// Answer with this, already complete.
    Reply(Value),
    /// Answer `initialize` with this, and give the client this session.
    Initialize {
        /// The answer.
        reply: Value,
        /// What was negotiated.
        session: Session,
    },
    /// A notification, or a response this server didn't ask for: answer `202 Accepted`.
    Accept,
    /// The client's answer to an elicitation this server asked: answer `202 Accepted` once
    /// it is handed on.
    Answer {
        /// The elicitation's ID, starting [`ELICIT`].
        id: String,
        /// The client's `result`, or its `error` as `{ "action": "cancel" }`.
        result: Value,
    },
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
                "_meta": { "dev.pigeon/view": o.view.name() },
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
        return match message.get("id").and_then(Value::as_str) {
            Some(i) if i.starts_with(ELICIT) => Action::Answer {
                id: i.to_string(),
                result: message
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| json!({ "action": "cancel" })),
            },
            _ => Action::Accept,
        };
    };
    if message.get("id").is_none() {
        return Action::Accept;
    }
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let session = negotiate(&params);
            Action::Initialize {
                reply: result(
                    &id,
                    &json!({
                        "protocolVersion": session.protocol,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "pigeon", "version": version },
                        "instructions": "pigeon's control plane. Propose a change, push its branch, read its status; the owner approves on the approval page this server links to, with a passkey.",
                        "_meta": { "dev.pigeon/contract": crate::view::CONTRACT },
                    }),
                ),
                session,
            }
        }
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

/// The tool error prefix of a presented view that does not match its operation's declared view.
pub const MISMATCH: &str = "view.mismatch";

/// The answer to a [`Action::Call`]: the operation's response as the tool's result, with
/// violations and errors as a tool error the model can read.
#[must_use]
pub fn answer(id: &Value, outcome: Result<Response, Error>) -> Value {
    match outcome {
        Ok(r) => {
            let view = if r.ok && crate::present::presented().contains(&r.operation.as_str()) {
                catalog().iter().find(|o| o.name == r.operation).map(|o| {
                    crate::present::checked(o.view, crate::present::view(&r.operation, &r))
                })
            } else {
                None
            };
            let text = serde_json::to_string_pretty(&r).unwrap_or_default();
            match view {
                Some(Err(problems)) => result(
                    id,
                    &json!({
                        "content": [{ "type": "text", "text": format!("{MISMATCH}: {}", problems.join("; ")) }],
                        "isError": true,
                    }),
                ),
                Some(Ok(Some(v))) => result(
                    id,
                    &json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": r,
                        "isError": !r.ok,
                        "_meta": { "dev.pigeon/view": v },
                    }),
                ),
                _ => result(
                    id,
                    &json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": r,
                        "isError": !r.ok,
                    }),
                ),
            }
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

/// One JSON-RPC message as a Server-Sent Event.
#[must_use]
pub fn sse(message: &Value) -> String {
    format!("event: message\ndata: {message}\n\n")
}
