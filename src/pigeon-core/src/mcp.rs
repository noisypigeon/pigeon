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
    /// A resource URI this server doesn't serve.
    pub const RESOURCE_NOT_FOUND: i64 = -32002;
}

/// The feed's resource; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §3.
pub const FEED_URI: &str = "pigeon://feed";

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
    /// Read this resource by running `request`, then [`contents`] with its outcome.
    Read {
        /// The request's ID.
        id: Value,
        /// The resource's URI.
        uri: String,
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
            let mut tool = json!({
                "name": tool_name(o),
                "title": o.name,
                "description": o.summary,
                "inputSchema": o.input_schema(),
                "annotations": {
                    "readOnlyHint": o.effect == crate::wire::Effect::Read,
                    "destructiveHint": o.effect == crate::wire::Effect::Apply,
                },
                "_meta": { "dev.pigeon/view": o.view.name() },
            });
            if let Some(out) = o.output_schema() {
                tool["outputSchema"] = out;
            }
            tool
        })
        .collect();
    json!({ "tools": tools })
}

/// Every resource.
#[must_use]
pub fn resources() -> Value {
    json!({ "resources": [{
        "uri": FEED_URI,
        "name": "feed",
        "title": "pigeon's feed",
        "description": "Items for the reader, newest last: read with ?after=<n|session>&needs=<owner|agent>&open=1&session=<label>",
        "mimeType": "application/json",
    }] })
}

/// What a `resources/read` with these `params` asks.
fn read(id: Value, params: &Value) -> Action {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return Action::Reply(error(&id, code::INVALID_PARAMS, "`uri` must be a string"));
    };
    let query = match uri.strip_prefix(FEED_URI) {
        Some("") => Some(""),
        Some(rest) => rest.strip_prefix('?'),
        None => None,
    };
    let Some(query) = query else {
        return Action::Reply(error(
            &id,
            code::RESOURCE_NOT_FOUND,
            &format!("no resource `{uri}`"),
        ));
    };
    let mut args = serde_json::Map::new();
    for k in ["after", "session", "needs", "open"] {
        if let Some(v) = crate::approve::field(query, k).filter(|v| !v.trim().is_empty()) {
            args.insert(k.into(), Value::String(v));
        }
    }
    args.insert("seconds".into(), Value::String("0".into()));
    Action::Read {
        id,
        uri: uri.to_string(),
        request: Request {
            operation: "feed.read".into(),
            args,
        },
    }
}

/// The owner's proof tools, listed only on a person's door; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` §4.
pub const PROOF_TOOLS: [&str; 3] = ["proof_challenge", "proof_submit", "proof_form"];

/// The proof tools' definitions, each a card.
#[must_use]
pub fn proof_tools() -> Vec<Value> {
    let page = json!({
        "type": "string",
        "description": "The approval link's path and query, as `/approve/12?grant=…`",
    });
    let purpose = json!({
        "type": "string",
        "enum": ["auto", "auto-end"],
        "description": "Instead of `page`: start (`auto`) or end (`auto-end`) the Auto window",
    });
    let text = json!({ "type": "string" });
    vec![
        json!({
            "name": PROOF_TOOLS[0],
            "title": "proof.challenge",
            "description": "What the grant at `page` approves, from the grant itself, or what `purpose` does, and the WebAuthn request options for the owner's passkey; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §4",
            "inputSchema": {
                "type": "object",
                "properties": { "page": page, "purpose": purpose },
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
            "_meta": { "dev.pigeon/view": "card" },
        }),
        json!({
            "name": PROOF_TOOLS[1],
            "title": "proof.submit",
            "description": "The owner's passkey assertion for the grant at `page`, with the page's other `fields` (as `operation`), or for `purpose`; the grant is redeemed and its operation run, or the Auto window started or ended; any other string or boolean argument is one of the page's fields; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §4",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "page": page,
                    "purpose": purpose,
                    "fields": { "type": "object", "additionalProperties": { "type": "string" } },
                    "credential": text,
                    "client_data": text,
                    "authenticator_data": text,
                    "signature": text,
                },
                "required": ["credential", "client_data", "authenticator_data", "signature"],
                "additionalProperties": { "type": ["string", "boolean"] },
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false },
            "_meta": { "dev.pigeon/view": "card" },
        }),
        json!({
            "name": PROOF_TOOLS[2],
            "title": "proof.form",
            "description": "The inputs the grant at `page` asks for (asks, review, a token) as a form whose submit is proof.submit; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md, its 2026-11-18 amendment §2",
            "inputSchema": {
                "type": "object",
                "properties": { "page": page },
                "required": ["page"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
            "_meta": { "dev.pigeon/view": "form" },
        }),
    ]
}

/// `reply`, a `tools/list` answer, with the proof tools added.
#[must_use]
pub fn with_proof_tools(mut reply: Value) -> Value {
    if let Some(tools) = reply
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
    {
        tools.extend(proof_tools());
    }
    reply
}

/// The proof tool `message` calls, with its ID and arguments; `None` for any other message.
#[must_use]
pub fn proof_call(message: &Value) -> Option<(Value, &'static str, Value)> {
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    let name = message.pointer("/params/name").and_then(Value::as_str)?;
    let tool = PROOF_TOOLS.into_iter().find(|t| *t == name)?;
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let args = message
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    Some((id, tool, args))
}

/// A proof tool's answer: `r` as its structured content, presented as a card titled `title`
/// from its scalar fields; or `why` as an error result.
#[must_use]
pub fn proof_answer(id: &Value, title: &str, outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(r) => {
            use crate::view::View;
            let presented = match title {
                "proof.challenge" => crate::present::proof::challenge(&r).map(|v| (View::Card, v)),
                "proof.form" => crate::present::proof::form(&r).map(|v| (View::Form, v)),
                _ => None,
            };
            let (declared, v) = presented.unwrap_or_else(|| {
                (
                    View::Card,
                    crate::present::card(title, None, crate::present::scalars(&r), Vec::new()),
                )
            });
            let view = crate::present::checked(declared, Some(v)).ok().flatten();
            let mut answer = json!({
                "content": [{ "type": "text", "text": r.to_string() }],
                "structuredContent": r,
                "isError": false,
            });
            if let Some(v) = view {
                answer["_meta"] = json!({ "dev.pigeon/view": v });
            }
            result(id, &answer)
        }
        Err(why) => result(
            id,
            &json!({ "content": [{ "type": "text", "text": why }], "isError": true }),
        ),
    }
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
                        "capabilities": {
                            "tools": { "listChanged": false },
                            "resources": { "subscribe": false, "listChanged": false },
                        },
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
        "resources/list" => Action::Reply(result(&id, &resources())),
        "resources/read" => read(id, &params),
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

/// The answer to a [`Action::Read`]: the operation's result as the resource's JSON text, or a JSON-RPC error.
#[must_use]
pub fn contents(id: &Value, uri: &str, outcome: Result<Response, Error>) -> Value {
    match outcome {
        Ok(r) if r.ok => result(
            id,
            &json!({ "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string(&r.result).unwrap_or_default(),
            }] }),
        ),
        Ok(r) => error(id, code::INVALID_PARAMS, &r.violations.join("; ")),
        Err(e) => error(id, code::INVALID_PARAMS, &e.to_string()),
    }
}

/// One JSON-RPC message as a Server-Sent Event.
#[must_use]
pub fn sse(message: &Value) -> String {
    format!("event: message\ndata: {message}\n\n")
}

#[cfg(test)]
mod proof_tests {
    use serde_json::json;

    use super::{PROOF_TOOLS, proof_answer, proof_call, proof_tools, with_proof_tools};

    #[test]
    fn proof_tools_are_cards_with_their_schemas() {
        let tools = proof_tools();
        let names: Vec<_> = tools.iter().map(|t| t["name"].clone()).collect();
        assert_eq!(
            names,
            [
                json!("proof_challenge"),
                json!("proof_submit"),
                json!("proof_form")
            ]
        );
        assert_eq!(tools[2]["_meta"]["dev.pigeon/view"], "form");
        for t in &tools[..2] {
            assert_eq!(t["_meta"]["dev.pigeon/view"], "card");
        }
        for t in &tools[..2] {
            let props = &t["inputSchema"]["properties"];
            assert!(props["page"].is_object() && props["purpose"].is_object());
            assert_eq!(props["purpose"]["enum"], json!(["auto", "auto-end"]));
        }
        assert!(tools[0]["inputSchema"]["required"].is_null());
        let submit = &tools[1]["inputSchema"]["required"];
        for k in [
            "credential",
            "client_data",
            "authenticator_data",
            "signature",
        ] {
            assert!(submit.as_array().unwrap().contains(&json!(k)), "{k}");
        }
    }

    #[test]
    fn the_catalog_lists_no_proof_tool() {
        let listed = super::tools();
        let names: Vec<_> = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(PROOF_TOOLS.iter().all(|p| !names.contains(p)));
    }

    #[test]
    fn a_persons_listing_adds_them() {
        let reply = json!({ "jsonrpc": "2.0", "id": 1, "result": { "tools": [{ "name": "x" }] } });
        let listed = with_proof_tools(reply);
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn only_a_proof_tool_call_is_one() {
        let call = |name: &str| {
            json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                    "params": { "name": name, "arguments": { "page": "/lane/1?grant=ab" } } })
        };
        let (id, tool, args) = proof_call(&call("proof_submit")).unwrap();
        assert_eq!((id, tool), (json!(7), "proof_submit"));
        assert_eq!(args["page"], "/lane/1?grant=ab");
        assert!(proof_call(&call("change_status")).is_none());
        let listing = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        assert!(proof_call(&listing).is_none());
    }

    #[test]
    fn an_answer_presents_a_valid_card() {
        let ok = proof_answer(
            &json!(1),
            "proof.challenge",
            Ok(json!({ "operation": "lane.grant", "target": "1" })),
        );
        assert_eq!(ok["result"]["_meta"]["dev.pigeon/view"]["view"], "card");
        assert_eq!(ok["result"]["isError"], false);
        let refused = proof_answer(&json!(1), "proof.submit", Err("no".into()));
        assert_eq!(refused["result"]["isError"], true);
    }
}
