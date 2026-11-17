//! The pool job's pure parts: its spec's environment, the prompt, the sandboxed commands, the stdio tools and its outcome; see `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md` §7 and its 2026-11-17 amendments.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The env key holding the task's id.
pub const TASK: &str = "POOL_TASK";
/// The env key holding the task's token.
pub const TOKEN: &str = "POOL_TASK_TOKEN";
/// The env key holding the gateway's URL.
pub const URL: &str = "POOL_URL";
/// The env key holding the Access client id.
pub const ACCESS_ID: &str = "POOL_ACCESS_ID";
/// The env key holding the Access client secret.
pub const ACCESS_SECRET: &str = "POOL_ACCESS_SECRET";
/// The env key holding the model.
pub const MODEL: &str = "POOL_MODEL";
/// The env key holding the brief, as JSON.
pub const BRIEF: &str = "POOL_BRIEF";
/// The env key holding the turn cap.
pub const TURNS: &str = "POOL_TURNS";
/// The env key holding the dollar cap.
pub const USD: &str = "POOL_USD";
/// The env key holding the minute cap.
pub const MINUTES: &str = "POOL_MINUTES";
/// The helper model for claude's small tasks.
pub const HELPER: &str = "claude-haiku-5-5";
/// The cap on claude's output tokens.
pub const MAX_OUTPUT: &str = "32000";
/// The tools claude may not use.
pub const DISALLOWED: &str = "WebSearch,WebFetch,Task";
/// The sandbox's `PATH`.
pub const SANDBOX_PATH: &str = "/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin";
/// The answer to `pool_escalate`.
pub const ESCALATED: &str =
    "Recorded. Stop now: the runner ends the task and hands your question to the planner.";
/// The user-namespace wrapper's argv.
pub const UNSHARE: [&str; 6] = [
    "unshare",
    "--user",
    "--map-users=100000,0,65536",
    "--map-groups=100000,0,65536",
    "--setuid=1000",
    "--setgid=1000",
];

/// A pool job's spec, parsed from its environment.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// The task's id.
    pub task: String,
    /// The task's token.
    pub token: String,
    /// The gateway's URL.
    pub url: String,
    /// The Access client id.
    pub access_id: String,
    /// The Access client secret.
    pub access_secret: String,
    /// The model.
    pub model: String,
    /// The brief.
    pub brief: Value,
    /// The turn cap.
    pub turns: u32,
    /// The dollar cap, as text.
    pub usd: String,
    /// The minute cap.
    pub minutes: u32,
}

/// How a pool job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The brief was carried out.
    Done,
    /// The model asked its planner.
    Escalated,
    /// The runner stopped it.
    Stopped,
    /// It failed.
    Failed,
}

/// What claude's stream-json output held.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The turns taken.
    pub turns: u32,
    /// The harness's cost, as its JSON text.
    pub cost: Option<String>,
    /// Whether the harness ended in an error.
    pub error: bool,
    /// The gateway's stop reason, if any.
    pub stop: Option<String>,
}

/// A pool job's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pooled {
    /// How it ended.
    pub outcome: Outcome,
    /// Why.
    pub why: String,
    /// The turns taken.
    pub turns: u32,
    /// The harness's cost, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_cost: Option<String>,
    /// Each check and whether it passed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<(String, bool)>,
    /// The escalation, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<Value>,
    /// The pushed commit, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed: Option<String>,
}

/// The job's spec from `env`.
///
/// # Errors
/// Names the first key that is missing or malformed.
pub fn parse(env: &BTreeMap<String, String>) -> Result<Job, String> {
    let mut vals = Vec::new();
    for key in [
        TASK,
        TOKEN,
        URL,
        ACCESS_ID,
        ACCESS_SECRET,
        MODEL,
        BRIEF,
        TURNS,
        USD,
        MINUTES,
    ] {
        match env.get(key) {
            Some(v) if !v.is_empty() => vals.push(v.clone()),
            _ => return Err(format!("pool job: `{key}` is missing")),
        }
    }
    let [
        task,
        token,
        url,
        access_id,
        access_secret,
        model,
        brief,
        turns,
        usd,
        minutes,
    ]: [String; 10] = vals.try_into().map_err(|_| String::new())?;
    if !url.starts_with("https://") {
        return Err(format!("pool job: `{URL}` is not https"));
    }
    let number = |key: &str, text: &str| -> Result<u32, String> {
        match text.parse::<u32>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("pool job: `{key}` is not a positive number")),
        }
    };
    let turns = number(TURNS, &turns)?;
    let minutes = number(MINUTES, &minutes)?;
    if !matches!(crate::pool::micros(&usd), Some(m) if m > 0) {
        return Err(format!("pool job: `{USD}` is not a dollar amount"));
    }
    let brief: Value = serde_json::from_str(&brief)
        .ok()
        .filter(|b: &Value| b.get("steps").is_some_and(Value::is_array))
        .ok_or_else(|| format!("pool job: `{BRIEF}` is not a brief"))?;
    Ok(Job {
        task,
        token,
        url,
        access_id,
        access_secret,
        model,
        brief,
        turns,
        usd,
        minutes,
    })
}

/// The executor's prompt for `job`.
#[must_use]
pub fn prompt(job: &Job) -> String {
    let title = job
        .brief
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("this brief");
    let pretty = serde_json::to_string_pretty(&job.brief).unwrap_or_default();
    format!(
        "You are an executor for pigeon, carrying out the brief \"{title}\" exactly. The brief, as JSON, is below; `brief_show` returns it too.\n\nRules:\n1. Do the brief's steps in order, and only them, touching only each step's files.\n2. After each step, check its `done_when`; if it fails, try once more within the step.\n3. On a second failure, a `stop_if`, or anything the brief doesn't cover, call `pool_escalate` with your question, what you tried and the evidence, then stop.\n4. Never commit, push, or call a tool but your file tools, `brief_show` and `pool_escalate`; the runner runs the brief's checks, commits and pushes when you exit.\n5. Never change the brief's decisions or widen its scope.\n\nBrief:\n{pretty}"
    )
}

/// Claude's environment for `job`, with `home` as its home.
#[must_use]
pub fn claude_env(job: &Job, home: &str) -> Vec<(String, String)> {
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    vec![
        pair("HOME", home),
        pair("PATH", SANDBOX_PATH),
        pair("ANTHROPIC_BASE_URL", &job.url),
        pair("ANTHROPIC_API_KEY", &job.token),
        (
            "ANTHROPIC_CUSTOM_HEADERS".to_string(),
            format!(
                "cf-access-client-id: {}\ncf-access-client-secret: {}",
                job.access_id, job.access_secret
            ),
        ),
        pair("ANTHROPIC_MODEL", &job.model),
        pair("ANTHROPIC_DEFAULT_HAIKU_MODEL", HELPER),
        pair("CLAUDE_CODE_MAX_OUTPUT_TOKENS", MAX_OUTPUT),
        pair("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        pair("DISABLE_AUTOUPDATER", "1"),
    ]
}

/// Claude's argv for `job`, with its MCP config.
#[must_use]
pub fn claude_argv(job: &Job, mcp_config: &str) -> Vec<String> {
    [
        "claude",
        "-p",
        &prompt(job),
        "--max-turns",
        &job.turns.to_string(),
        "--max-budget-usd",
        &job.usd,
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode",
        "bypassPermissions",
        "--disallowedTools",
        DISALLOWED,
        "--mcp-config",
        mcp_config,
        "--strict-mcp-config",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// A brief check's environment.
#[must_use]
pub fn check_env(home: &str, cargo_home: &str) -> Vec<(String, String)> {
    vec![
        ("HOME".to_string(), home.to_string()),
        ("PATH".to_string(), SANDBOX_PATH.to_string()),
        ("CARGO_HOME".to_string(), cargo_home.to_string()),
    ]
}

/// A brief check's argv.
#[must_use]
pub fn check_argv(check: &str) -> Vec<String> {
    vec!["sh".to_string(), "-c".to_string(), check.to_string()]
}

/// Whether the pool can run `check`.
#[must_use]
pub fn runnable(check: &str) -> bool {
    !(check.contains("artifacts/") || check.starts_with("rustup "))
}

/// `argv` under `env`, wrapped in the user-namespace sandbox.
#[must_use]
pub fn sandboxed(env: &[(String, String)], argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = ["sh", "-c", "umask 000 && exec \"$@\"", "sh"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    out.extend(UNSHARE.iter().map(|s| (*s).to_string()));
    out.push("env".to_string());
    out.push("-i".to_string());
    out.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
    out.extend(argv.iter().cloned());
    out
}

/// The MCP config running `pigeon agent pool-tools`.
#[must_use]
pub fn mcp_config(pigeon: &str, brief: &str, escalation: &str) -> String {
    json!({"mcpServers": {"pool": {
        "command": pigeon,
        "args": ["agent", "pool-tools", "--brief", brief, "--escalation", escalation],
    }}})
    .to_string()
}

/// What claude's stream-json `stdout` held.
#[must_use]
pub fn stream(stdout: &str) -> Stream {
    let mut s = Stream::default();
    let result = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v.is_object() && v.get("type").and_then(Value::as_str) == Some("result"));
    if let Some(r) = result {
        s.turns = r
            .get("num_turns")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        s.cost = r
            .get("total_cost_usd")
            .filter(|c| c.is_number())
            .map(ToString::to_string);
        s.error = r.get("is_error") == Some(&Value::Bool(true));
    }
    if let Some(at) = stdout.find("pool: ") {
        let rest = &stdout[at..];
        let end = rest.find(['"', '\\', '\n']).unwrap_or(rest.len());
        s.stop = Some(rest[..end].to_string());
    }
    s
}

/// How the job ended, and why.
#[must_use]
pub fn outcome(escalated: bool, timed_out: bool, s: &Stream, checks_ok: bool) -> (Outcome, String) {
    if escalated {
        (Outcome::Escalated, "the model escalated".to_string())
    } else if timed_out {
        (Outcome::Stopped, "minutes".to_string())
    } else if let Some(t) = &s.stop {
        (Outcome::Stopped, t.clone())
    } else if s.error {
        (Outcome::Failed, "the harness ended in an error".to_string())
    } else if !checks_ok {
        (Outcome::Failed, "a check failed".to_string())
    } else {
        (Outcome::Done, "done".to_string())
    }
}

/// The commit's message, `wip` when the work is unfinished.
#[must_use]
pub fn commit_message(job: &Job, scope: &[String], wip: bool) -> String {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let commit = job.brief.get("commit");
    let mut title = text(commit.and_then(|c| c.get("title")))
        .or_else(|| text(job.brief.get("title")))
        .unwrap_or_else(|| "pool task".to_string());
    if wip {
        title = format!("wip: {title}");
    }
    let plan = text(commit.and_then(|c| c.get("plan"))).unwrap_or_default();
    let mut lines: Vec<String> = scope.iter().map(|s| format!("Change-Scope: {s}")).collect();
    lines.push(format!("Pool-Task: {}", job.task));
    format!("{title}\n\n{plan}\n\n{}", lines.join("\n"))
}

/// The tools `pool-tools` serves.
#[must_use]
pub fn tools() -> Value {
    json!([
        {
            "name": "brief_show",
            "description": "The brief you carry out, as JSON.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        },
        {
            "name": "pool_escalate",
            "description": "Stop and ask your planner: your question, what you tried, and the evidence. Then stop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "tried": {"type": "string"},
                    "evidence": {"type": "string"},
                },
                "required": ["question", "tried"],
                "additionalProperties": false,
            },
        },
    ])
}

fn reply(id: &Value, result: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn failure(id: &Value, code: i32, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

fn text_result(text: &str, is_error: bool) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": text}]});
    if is_error {
        r["isError"] = Value::Bool(true);
    }
    r
}

/// The JSON-RPC answer to `line`, and the escalation it recorded, if any.
#[must_use]
pub fn answer(line: &str, brief: &str, version: &str) -> (Option<String>, Option<Value>) {
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return (Some(failure(&Value::Null, -32700, "parse error")), None);
    };
    let Some(id) = msg.get("id") else {
        return (None, None);
    };
    let params = msg.get("params");
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => {
            let proto = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18");
            let result = json!({
                "protocolVersion": proto,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "pool", "version": version},
            });
            (Some(reply(id, &result)), None)
        }
        "tools/list" => (Some(reply(id, &json!({"tools": tools()}))), None),
        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match name {
                "brief_show" => (Some(reply(id, &text_result(brief, false))), None),
                "pool_escalate" => {
                    let arg = |k: &str| {
                        params
                            .and_then(|p| p.get("arguments"))
                            .and_then(|a| a.get(k))
                            .and_then(Value::as_str)
                    };
                    if let (Some(question), Some(tried)) = (arg("question"), arg("tried")) {
                        let escalation = json!({
                            "question": question,
                            "tried": tried,
                            "evidence": arg("evidence").unwrap_or(""),
                        });
                        (
                            Some(reply(id, &text_result(ESCALATED, false))),
                            Some(escalation),
                        )
                    } else {
                        let r = text_result("pool_escalate needs `question` and `tried`", true);
                        (Some(reply(id, &r)), None)
                    }
                }
                other => {
                    let r = text_result(&format!("no tool `{other}`"), true);
                    (Some(reply(id, &r)), None)
                }
            }
        }
        other => (
            Some(failure(id, -32601, &format!("no method `{other}`"))),
            None,
        ),
    }
}

#[cfg(test)]
mod tests;
