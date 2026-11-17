//! provider.call and provider.tools: MCP calls to providers through the broker's verdict; see .claude/decisions/2026-11-16-security-provider-calls-mcp-broker.md.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use worker::{Date, Env, Fetch, Headers, Method, Request, RequestInit, Result};

use super::platform::{self, Settings};
use super::{events, tickets, to_change};
use crate::access::Identity;
use crate::control;
use crate::core::provider::call::{self, Args};
use crate::core::provider::client::{self, ClientError};
use crate::core::provider::policy;
use crate::core::provider::registry::{self, Credential, Effect, Server};
use crate::core::provider::verdict::{Call, Caller, ChangeState, Decision, Grants, Step, verdict};

const NOT_WIRED: &str = "credential kind or server not wired yet; see the broker ADR's step 7";

fn violations(status: u16, why: impl Into<String>) -> (u16, Value) {
    (status, json!({ "violations": [why.into()] }))
}

/// One MCP answer: its session header, content type and body.
struct Answer {
    session: Option<String>,
    content_type: String,
    body: String,
}

/// POSTs one JSON-RPC message to `server`.
async fn post(
    server: &Server,
    token: &str,
    session: Option<&str>,
    message: &Value,
) -> std::result::Result<Answer, String> {
    let headers = Headers::new();
    for (k, v) in call::headers(server, Some(token), session) {
        headers.set(&k, &v).map_err(|e| e.to_string())?;
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_headers(headers)
        .with_body(Some(message.to_string().into()));
    let req = Request::new_with_init(&server.url, &init).map_err(|e| e.to_string())?;
    let mut r = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status_code();
    let session = r.headers().get("mcp-session-id").ok().flatten();
    let content_type = r
        .headers()
        .get("content-type")
        .ok()
        .flatten()
        .unwrap_or_default();
    let body = r.text().await.map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("provider answered {status}"));
    }
    Ok(Answer {
        session,
        content_type,
        body,
    })
}

/// Initializes the server's session and acknowledges it: its session id, if it gave one.
async fn open(server: &Server, token: &str) -> std::result::Result<Option<String>, String> {
    let init = post(server, token, None, &client::initialize(1)).await?;
    client::response(&init.body, &init.content_type, 1).map_err(|e| e.to_string())?;
    let session = init.session;
    post(server, token, session.as_deref(), &client::initialized()).await?;
    Ok(session)
}

/// Records `provider.refused` with `summary`, and answers `status` with `reason`.
async fn refuse(
    env: &Env,
    s: &Settings,
    change: Option<u32>,
    label: Option<String>,
    summary: &str,
    status: u16,
    reason: impl Into<String>,
) -> Result<(u16, Value)> {
    events::append(env, s, "provider.refused", change, label, summary, false).await?;
    Ok(violations(status, reason))
}

/// Why a provider-call policy refuses the call, naming the policy, or none.
fn policy_refusal(sent: &Call, effect: Effect, caller: &str) -> Option<String> {
    match policy::judge(&policy::document(sent, effect, caller)) {
        Ok(found) => found.map(|(name, why)| format!("{name}: {why}")),
        Err((name, why)) => Some(format!("{name} doesn't parse: {why}")),
    }
}

fn secret_name(server: &Server) -> Option<String> {
    if server.url.starts_with("inproc:") {
        return None;
    }
    match Credential::parse(&server.credential) {
        Some(Credential::Secret(name)) => Some(name),
        _ => None,
    }
}

/// Runs `provider.call` or `provider.tools`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    op: &str,
    args: &Value,
) -> Result<(u16, Value)> {
    match op {
        "provider.call" => provider_call(env, s, who, args).await,
        "provider.tools" => provider_tools(env, args).await,
        other => Ok((404, json!(format!("`{other}` is not a provider operation")))),
    }
}

async fn provider_call(
    env: &Env,
    s: &Settings,
    who: &Identity,
    args: &Value,
) -> Result<(u16, Value)> {
    let asked: Args = match call::args(&json!({ "args": args })) {
        Ok(found) => found,
        Err(msg) => return Ok(violations(400, msg)),
    };
    let registry = match registry::parse(registry::CONFIG) {
        Ok(r) => r,
        Err(messages) => return Ok((500, json!({ "violations": messages }))),
    };

    // The ticket.
    let ticket = args
        .get("ticket")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    let (label, refusal) = match ticket {
        None => (None, None),
        Some(t) => match tickets::holder(env, s, Some(t)).await? {
            Ok(label) => (Some(label), None),
            Err(refusal) => (None, Some(refusal)),
        },
    };
    let live = call::ticket(ticket.is_some(), refusal.as_deref());

    // The holds.
    let holds = held(env, s, label.as_deref()).await?;

    // The change.
    let (state, write) = change_facts(env, s, asked.change, &holds).await?;

    let caller = Caller {
        account: who.subject.clone(),
        platform: false,
        capabilities: call::capabilities(false, who.human, live),
        ticket: live,
        holds,
    };
    let sent = call::call(&asked, state);
    let change = asked.change.and_then(|n| u32::try_from(n).ok());
    let decision = verdict(
        &registry,
        &sent,
        &caller,
        None,
        Grants {
            write,
            proof: false,
        },
    );
    let (effect, credential) = match decision {
        Decision::Refused { step, reason } => {
            let summary = call::refused(&sent, step, &reason);
            return refuse(env, s, change, label, &summary, 403, reason).await;
        }
        Decision::Pending { reason } => return Ok((200, json!({ "pending": reason }))),
        Decision::Forward {
            effect, credential, ..
        } => (effect, credential),
    };

    // The policies.
    if let Some(reason) = policy_refusal(&sent, effect, &who.subject) {
        let summary = call::refused(&sent, Step::Policy, &reason);
        return refuse(env, s, change, label, &summary, 403, reason).await;
    }

    // The credential.
    let server = &registry.servers[&asked.server];
    let (Credential::Secret(name), Some(_)) = (&credential, secret_name(server)) else {
        let summary = call::refused(&sent, Step::Registry, NOT_WIRED);
        return refuse(env, s, change, label, &summary, 501, NOT_WIRED).await;
    };
    let token = platform::secret(env, name)?;

    // The exchange.
    let started = Date::now().as_millis();
    let outcome = exchange(server, &token, &asked).await;
    let millis = Date::now().as_millis().saturating_sub(started);
    let word = if outcome.is_ok() { "ok" } else { "error" };
    events::append(
        env,
        s,
        "provider.called",
        change,
        label,
        &call::called(&sent, effect, word, millis),
        false,
    )
    .await?;
    Ok(match outcome {
        Ok(value) => (200, json!({ "result": value })),
        Err(why) => violations(502, why),
    })
}

async fn exchange(
    server: &Server,
    token: &str,
    asked: &Args,
) -> std::result::Result<Value, String> {
    let [_, _, request] = call::exchange(&asked.tool, &asked.arguments);
    let session = open(server, token).await?;
    let answer = post(server, token, session.as_deref(), &request).await?;
    let result = client::response(&answer.body, &answer.content_type, 2)
        .map_err(|e: ClientError| e.to_string())?;
    client::tool_result(&result).map_err(|e| e.to_string())
}

async fn provider_tools(env: &Env, args: &Value) -> Result<(u16, Value)> {
    let registry = match registry::parse(registry::CONFIG) {
        Ok(r) => r,
        Err(messages) => return Ok((500, json!({ "violations": messages }))),
    };
    let name = args.get("server").and_then(Value::as_str).unwrap_or("");
    let Some(server) = registry.servers.get(name) else {
        return Ok(violations(
            404,
            format!("server `{name}` is not in the registry"),
        ));
    };
    let Some(secret) = secret_name(server) else {
        return Ok(violations(501, NOT_WIRED));
    };
    let token = platform::secret(env, &secret)?;
    let live = match list(server, &token).await {
        Ok(l) => l,
        Err(why) => return Ok(violations(502, why)),
    };
    let tools: Vec<Value> = live
        .into_iter()
        .map(|(tool, hash)| {
            let pinned = server.tools.get(&tool).map(|t| t.schema.clone());
            let drift = pinned.as_ref().is_some_and(|p| *p != hash);
            json!({ "name": tool, "live": hash, "pinned": pinned, "drift": drift })
        })
        .collect();
    Ok((200, json!({ "tools": tools })))
}

/// Every live tool's name and schema hash, sorted by name.
async fn list(server: &Server, token: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let session = open(server, token).await?;
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    let mut id = 2;
    loop {
        let request = client::tools_list(id, cursor.as_deref());
        let answer = post(server, token, session.as_deref(), &request).await?;
        let result =
            client::response(&answer.body, &answer.content_type, id).map_err(|e| e.to_string())?;
        let page = client::listed(&result).map_err(|e| e.to_string())?;
        all.extend(page.tools);
        match page.next {
            Some(next) => {
                cursor = Some(next);
                id += 1;
            }
            None => break,
        }
    }
    all.sort();
    Ok(all)
}

/// The named change's state, and whether a write is covered: held, with its plan approved.
async fn change_facts(
    env: &Env,
    s: &Settings,
    named: Option<u64>,
    holds: &BTreeSet<u64>,
) -> Result<(Option<ChangeState>, bool)> {
    let Some(n) = named else {
        return Ok((None, false));
    };
    let number = u32::try_from(n).unwrap_or(0);
    let (status, reply) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
    let change = (status == 200)
        .then(|| {
            serde_json::from_value::<crate::core::change::Change>(reply["change"].clone()).ok()
        })
        .flatten();
    let state = match &change {
        Some(found) => {
            let word = serde_json::to_value(found.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            call::change_state(&word, found.evicted.is_some())
        }
        None => ChangeState::Closed,
    };
    let write = holds.contains(&n) && change.is_some_and(|ch| ch.plan_approval.is_some());
    Ok((Some(state), write))
}

/// The numbers of the open changes `label`'s session holds.
async fn held(env: &Env, s: &Settings, label: Option<&str>) -> Result<BTreeSet<u64>> {
    let Some(label) = label else {
        return Ok(BTreeSet::new());
    };
    let rows = platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?;
    Ok(control::open_rows(&rows)
        .into_iter()
        .filter(|o| o.session.as_deref() == Some(label))
        .map(|o| u64::from(o.number))
        .collect())
}
