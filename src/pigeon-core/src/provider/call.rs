//! A brokered call's pieces: its arguments, its caller, the request it sends and the events it records.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::client;
use super::registry::{Effect, Server};
use super::verdict::{Call, ChangeState, Step, Ticket};

/// What `provider.call` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The registry server's name.
    pub server: String,
    /// The tool's name.
    pub tool: String,
    /// The tool's arguments, a JSON object.
    pub arguments: Value,
    /// The change the call names, if any.
    pub change: Option<u64>,
}

/// Reads a request's `args` object.
///
/// # Errors
/// What is missing or malformed.
pub fn args(request: &Value) -> Result<Args, String> {
    let a = request.get("args").unwrap_or(&Value::Null);
    let text = |k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let (Some(server), Some(tool)) = (text("server"), text("tool")) else {
        return Err("provider.call needs `server` and `tool`".into());
    };
    let arguments = match a.get("arguments").and_then(Value::as_str) {
        None => json!({}),
        Some(t) => serde_json::from_str::<Value>(t)
            .ok()
            .filter(Value::is_object)
            .ok_or("arguments is JSON text of an object")?,
    };
    let change = match a.get("change").and_then(Value::as_str) {
        None => None,
        Some(c) => Some(
            c.strip_prefix('!')
                .unwrap_or(c)
                .parse::<u64>()
                .map_err(|_| "change is a change number")?,
        ),
    };
    Ok(Args {
        server,
        tool,
        arguments,
        change,
    })
}

/// The ticket's state from whether one was given and why it was refused.
#[must_use]
pub fn ticket(given: bool, refusal: Option<&str>) -> Ticket {
    match (given, refusal) {
        (false, _) => Ticket::Missing,
        (true, None) => Ticket::Live,
        (true, Some(r)) if r.contains("expired") => Ticket::Lapsed,
        (true, Some(_)) => Ticket::Revoked,
    }
}

/// The caller's capabilities.
#[must_use]
pub fn capabilities(platform: bool, human: bool, ticket: Ticket) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = ["read"].map(String::from).into();
    if platform {
        set.extend(["propose", "platform"].map(String::from));
    } else if human || ticket == Ticket::Live {
        set.insert("propose".into());
    }
    set
}

/// A change's state from its record.
#[must_use]
pub fn change_state(state: &str, evicted: bool) -> ChangeState {
    if evicted {
        return ChangeState::Evicted;
    }
    match state {
        "merged" => ChangeState::Merged,
        "closed" => ChangeState::Closed,
        _ => ChangeState::Open,
    }
}

/// The verdict's call, the change defaulting to open.
#[must_use]
pub fn call(a: &Args, state: Option<ChangeState>) -> Call {
    Call {
        server: a.server.clone(),
        tool: a.tool.clone(),
        arguments: a.arguments.clone(),
        change: a.change.map(|n| (n, state.unwrap_or(ChangeState::Open))),
    }
}

/// The MCP request's headers, in order.
#[must_use]
pub fn headers(
    server: &Server,
    token: Option<&str>,
    session: Option<&str>,
) -> Vec<(String, String)> {
    let mut h = vec![
        ("content-type".to_string(), "application/json".to_string()),
        (
            "accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
        (
            "mcp-protocol-version".to_string(),
            client::PROTOCOL.to_string(),
        ),
    ];
    h.extend(server.headers.iter().map(|(k, v)| (k.clone(), v.clone())));
    if let Some(t) = token {
        h.push(("authorization".into(), format!("Bearer {t}")));
    }
    if let Some(s) = session {
        h.push(("mcp-session-id".into(), s.to_string()));
    }
    h
}

/// The three requests of a call: initialize, initialized, then the tool.
#[must_use]
pub fn exchange(tool: &str, arguments: &Value) -> [Value; 3] {
    [
        client::initialize(1),
        client::initialized(),
        client::tools_call(2, tool, arguments),
    ]
}

/// The event summary of a call made; the arguments only hashed.
///
/// # Panics
/// Never: a `Value` always serializes.
#[must_use]
pub fn called(call: &Call, effect: Effect, outcome: &str, millis: u64) -> String {
    let hash = common::hex::sha256_hex(
        serde_json::to_string(&call.arguments)
            .expect("serializes")
            .as_bytes(),
    );
    format!(
        "{}/{} {} {outcome} in {millis} ms; arguments {}",
        call.server,
        call.tool,
        effect.name(),
        &hash[..16]
    )
}

/// The event summary of a call refused.
#[must_use]
pub fn refused(call: &Call, step: Step, reason: &str) -> String {
    format!(
        "{}/{} refused at {}: {reason}",
        call.server,
        call.tool,
        step.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::registry;

    fn req(args: &Value) -> Value {
        json!({ "args": args })
    }

    #[test]
    fn args_reads_a_call() {
        let a = args(&req(
            &json!({"server": "s", "tool": "t", "arguments": "{\"x\":1}", "change": "!12"}),
        ))
        .expect("reads");
        assert_eq!(a.server, "s");
        assert_eq!(a.tool, "t");
        assert_eq!(a.arguments, json!({"x": 1}));
        assert_eq!(a.change, Some(12));
    }

    #[test]
    fn args_defaults_arguments_to_an_empty_object() {
        let a = args(&req(&json!({"server": "s", "tool": "t"}))).expect("reads");
        assert_eq!(a.arguments, json!({}));
        assert_eq!(a.change, None);
    }

    #[test]
    fn args_refuses_bad_input() {
        let bad = [
            json!({"tool": "t"}),
            json!({"server": "s", "tool": ""}),
            json!({"server": "s", "tool": "t", "arguments": "[1]"}),
            json!({"server": "s", "tool": "t", "arguments": "not json"}),
            json!({"server": "s", "tool": "t", "change": "x"}),
        ];
        for b in bad {
            assert!(args(&req(&b)).is_err(), "{b}");
        }
    }

    #[test]
    fn ticket_maps_each_answer() {
        assert_eq!(ticket(false, None), Ticket::Missing);
        assert_eq!(ticket(true, None), Ticket::Live);
        assert_eq!(ticket(true, Some("ticket expired")), Ticket::Lapsed);
        assert_eq!(ticket(true, Some("ticket revoked")), Ticket::Revoked);
    }

    #[test]
    fn capabilities_by_caller() {
        let set = |v: &[&str]| v.iter().map(ToString::to_string).collect::<BTreeSet<_>>();
        assert_eq!(
            capabilities(true, false, Ticket::Missing),
            set(&["read", "propose", "platform"])
        );
        assert_eq!(
            capabilities(false, true, Ticket::Missing),
            set(&["read", "propose"])
        );
        assert_eq!(
            capabilities(false, false, Ticket::Live),
            set(&["read", "propose"])
        );
        assert_eq!(capabilities(false, false, Ticket::Lapsed), set(&["read"]));
    }

    #[test]
    fn change_state_from_the_record() {
        assert_eq!(change_state("closed", true), ChangeState::Evicted);
        assert_eq!(change_state("merged", false), ChangeState::Merged);
        assert_eq!(change_state("closed", false), ChangeState::Closed);
        assert_eq!(change_state("approved", false), ChangeState::Open);
    }

    #[test]
    fn call_carries_the_change_and_its_state() {
        let mut a = Args {
            server: "s".into(),
            tool: "t".into(),
            arguments: json!({}),
            change: Some(7),
        };
        assert_eq!(call(&a, None).change, Some((7, ChangeState::Open)));
        assert_eq!(
            call(&a, Some(ChangeState::Merged)).change,
            Some((7, ChangeState::Merged))
        );
        a.change = None;
        assert_eq!(call(&a, Some(ChangeState::Merged)).change, None);
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn server() -> Server {
        let yaml = format!(
            "servers:\n  bk:\n    url: https://mcp.example/mcp\n    credential: secret:BK_TOKEN\n    headers:\n      X-Buildkite-Toolsets: builds\n    tools:\n      get_build:\n        effect: read\n        capability: ci.read\n        schema: {A}\n"
        );
        registry::parse(&yaml).expect("parses").servers["bk"].clone()
    }

    #[test]
    fn headers_in_order_with_and_without_token_and_session() {
        let s = server();
        let h = |k: &str, v: &str| (k.to_string(), v.to_string());
        let base = vec![
            h("content-type", "application/json"),
            h("accept", "application/json, text/event-stream"),
            h("mcp-protocol-version", client::PROTOCOL),
            h("X-Buildkite-Toolsets", "builds"),
        ];
        assert_eq!(headers(&s, None, None), base);
        let mut full = base;
        full.push(h("authorization", "Bearer tok"));
        full.push(h("mcp-session-id", "sess"));
        assert_eq!(headers(&s, Some("tok"), Some("sess")), full);
    }

    #[test]
    fn exchange_initializes_then_calls() {
        let args = json!({"a": 1});
        let [i, n, c] = exchange("get_build", &args);
        assert_eq!(i, client::initialize(1));
        assert_eq!(n, client::initialized());
        assert_eq!(c, client::tools_call(2, "get_build", &args));
    }

    fn sample(arguments: Value) -> Call {
        Call {
            server: "bk".into(),
            tool: "get_build".into(),
            arguments,
            change: None,
        }
    }

    #[test]
    fn called_hashes_and_never_prints_arguments() {
        let s = called(&sample(json!({"k": "topsecret"})), Effect::Read, "ok", 12);
        assert!(!s.contains("topsecret"));
        assert!(s.starts_with("bk/get_build read ok in 12 ms; arguments "));
        let hex = s.split("arguments ").nth(1).expect("hash");
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn refused_names_the_step() {
        let step = Step::Registry;
        let s = refused(&sample(json!({})), step, "no such tool");
        assert_eq!(
            s,
            format!("bk/get_build refused at {}: no such tool", step.name())
        );
    }
}
