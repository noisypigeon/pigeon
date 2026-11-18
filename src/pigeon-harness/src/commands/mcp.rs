//! The harness's MCP client: an operation as a tool on `/mcp`; see
//! `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, 2026-11-18 amendment §6.

use std::sync::Mutex;

use serde_json::{Value, json};

/// The MCP session id, once `initialize` has answered.
static SESSION: Mutex<Option<String>> = Mutex::new(None);

/// A tool call's HTTP reply, its body the operation's `{operation, ok, result, violations}`.
pub(crate) struct Reply {
    pub status: u16,
    pub content_type: String,
    pub cloudflare: bool,
    pub body: String,
}

fn session() -> Option<String> {
    SESSION.lock().ok().and_then(|s| s.clone())
}

fn remember(id: Option<String>) {
    if let Ok(mut s) = SESSION.lock() {
        *s = id;
    }
}

fn post(
    agent: &ureq::Agent,
    url: &str,
    id: Option<&str>,
    body: &Value,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut request = agent
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json");
    if let Some(id) = id {
        request = request.header("mcp-session-id", id);
    }
    request.send(body.to_string()).map_err(|e| e.to_string())
}

fn initialize(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "pigeon-harness", "version": env!("CARGO_PKG_VERSION") },
        },
    });
    let response = post(agent, url, None, &body)?;
    let id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    remember(Some(id.clone()));
    Ok(id)
}

/// Runs `operation` as a tool on `host`'s `/mcp`.
pub(crate) fn call(
    agent: &ureq::Agent,
    host: &str,
    operation: &str,
    args: &Value,
) -> Result<Reply, String> {
    let url = format!("https://{host}/mcp");
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": operation.replace('.', "_"), "arguments": args },
    });
    let held = session();
    let had = held.is_some();
    let id = match held {
        Some(id) => id,
        None => initialize(agent, &url)?,
    };
    let mut response = post(agent, &url, Some(&id), &body)?;
    if had && response.status().as_u16() == 404 {
        remember(None);
        let id = initialize(agent, &url)?;
        response = post(agent, &url, Some(&id), &body)?;
    }
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let cloudflare = response.headers().contains_key("cf-ray");
    let raw = response.body_mut().read_to_string().unwrap_or_default();
    Ok(Reply {
        status,
        content_type,
        cloudflare,
        body: body_of(status, operation, raw),
    })
}

/// The reply's body: the tool's `structuredContent`, or its error as a violation, else `raw`.
fn body_of(status: u16, operation: &str, raw: String) -> String {
    if status != 200 {
        return raw;
    }
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return raw;
    };
    if v["result"]["isError"].as_bool() == Some(true) {
        let text = v["result"]["content"][0]["text"].clone();
        return json!({
            "operation": operation,
            "ok": false,
            "result": null,
            "violations": [text],
        })
        .to_string();
    }
    match v["result"].get("structuredContent") {
        Some(s) => s.to_string(),
        None => raw,
    }
}
