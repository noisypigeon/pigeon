use super::*;

const BRIEF_JSON: &str = r#"{"title":"t","steps":[],"checks":["cargo fmt --all -- --check","CHANGE_BASE=artifacts/main pigeon dev ci-scoped"],"commit":{"title":"feat(x): y","plan":"p"}}"#;

fn env() -> BTreeMap<String, String> {
    [
        (TASK, "b9.1"),
        (TOKEN, "tok"),
        (URL, "https://pool.example/v0/pool/anthropic"),
        (ACCESS_ID, "id"),
        (ACCESS_SECRET, "secret"),
        (MODEL, "claude-sonnet-5-5"),
        (BRIEF, BRIEF_JSON),
        (TURNS, "200"),
        (USD, "4"),
        (MINUTES, "40"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
    .collect()
}

fn job() -> Job {
    parse(&env()).unwrap()
}

fn with(key: &str, value: &str) -> BTreeMap<String, String> {
    let mut e = env();
    e.insert(key.to_string(), value.to_string());
    e
}

fn s(x: &str) -> String {
    x.to_string()
}

#[test]
fn parse_ok() {
    let j = job();
    assert_eq!(j.task, "b9.1");
    assert_eq!(j.token, "tok");
    assert_eq!(j.turns, 200);
    assert_eq!(j.usd, "4");
    assert_eq!(j.minutes, 40);
    assert_eq!(j.brief["title"], "t");
}

#[test]
fn parse_each_missing_key() {
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
        let mut e = env();
        e.remove(key);
        assert_eq!(parse(&e), Err(format!("pool job: `{key}` is missing")));
    }
}

#[test]
fn parse_empty_value() {
    assert_eq!(
        parse(&with(MODEL, "")),
        Err(s("pool job: `POOL_MODEL` is missing"))
    );
}

#[test]
fn parse_missing_reported_in_const_order() {
    let mut e = env();
    e.remove(MINUTES);
    e.remove(TASK);
    assert_eq!(parse(&e), Err(s("pool job: `POOL_TASK` is missing")));
}

#[test]
fn parse_http_url() {
    assert_eq!(
        parse(&with(URL, "http://pool.example")),
        Err(s("pool job: `POOL_URL` is not https"))
    );
}

#[test]
fn parse_bad_turns() {
    for v in ["0", "x"] {
        assert_eq!(
            parse(&with(TURNS, v)),
            Err(s("pool job: `POOL_TURNS` is not a positive number"))
        );
    }
}

#[test]
fn parse_bad_minutes() {
    for v in ["0", "x"] {
        assert_eq!(
            parse(&with(MINUTES, v)),
            Err(s("pool job: `POOL_MINUTES` is not a positive number"))
        );
    }
}

#[test]
fn parse_turns_checked_before_minutes() {
    let mut e = with(TURNS, "0");
    e.insert(s(MINUTES), s("0"));
    assert_eq!(
        parse(&e),
        Err(s("pool job: `POOL_TURNS` is not a positive number"))
    );
}

#[test]
fn parse_bad_usd() {
    for v in ["0", "abc"] {
        assert_eq!(
            parse(&with(USD, v)),
            Err(s("pool job: `POOL_USD` is not a dollar amount"))
        );
    }
}

#[test]
fn parse_usd_keeps_text() {
    assert_eq!(parse(&with(USD, "4.50")).unwrap().usd, "4.50");
}

#[test]
fn parse_brief_without_steps() {
    assert_eq!(
        parse(&with(BRIEF, r#"{"title":"t"}"#)),
        Err(s("pool job: `POOL_BRIEF` is not a brief"))
    );
}

#[test]
fn parse_brief_not_json_or_steps_not_array() {
    for v in ["nope", r#"{"steps":1}"#, "[]"] {
        assert_eq!(
            parse(&with(BRIEF, v)),
            Err(s("pool job: `POOL_BRIEF` is not a brief"))
        );
    }
}

#[test]
fn prompt_holds_title_and_rules() {
    let p = prompt(&job());
    assert!(p.contains("carrying out the brief \"t\" exactly"));
    for n in 1..=5 {
        assert!(p.contains(&format!("\n{n}. ")), "rule {n}");
    }
    assert!(p.contains("`pool_escalate`"));
    assert!(p.contains("Brief:\n{"));
    assert!(p.contains("\"steps\": []"));
}

#[test]
fn prompt_without_title() {
    let mut j = job();
    j.brief = json!({"steps": []});
    assert!(prompt(&j).contains("the brief \"this brief\" exactly"));
}

#[test]
fn claude_env_pairs_in_order() {
    let e = claude_env(&job(), "/home/x");
    let keys: Vec<&str> = e.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "HOME",
            "PATH",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_CUSTOM_HEADERS",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "DISABLE_AUTOUPDATER",
        ]
    );
    assert_eq!(e.len(), 10);
}

#[test]
fn claude_env_values() {
    let e = claude_env(&job(), "/home/x");
    assert_eq!(e[0].1, "/home/x");
    assert_eq!(e[1].1, SANDBOX_PATH);
    assert_eq!(e[2].1, "https://pool.example/v0/pool/anthropic");
    assert_eq!(e[3].1, "tok");
    assert_eq!(e[5].1, "claude-sonnet-5-5");
    assert_eq!(e[6].1, "claude-haiku-5-5");
    assert_eq!(e[7].1, "32000");
    assert_eq!(e[8].1, "1");
    assert_eq!(e[9].1, "1");
}

#[test]
fn claude_env_access_lines() {
    let e = claude_env(&job(), "/h");
    assert_eq!(
        e[4].1,
        "cf-access-client-id: id\ncf-access-client-secret: secret"
    );
}

#[test]
fn claude_argv_flags() {
    let a = claude_argv(&job(), "{}");
    assert_eq!(&a[..2], ["claude", "-p"]);
    assert_eq!(a[2], prompt(&job()));
    let pos = |f: &str| a.iter().position(|x| x == f).unwrap();
    assert_eq!(a[pos("--max-turns") + 1], "200");
    assert_eq!(a[pos("--max-budget-usd") + 1], "4");
    assert_eq!(a[pos("--output-format") + 1], "stream-json");
    assert_eq!(a[pos("--permission-mode") + 1], "bypassPermissions");
    assert_eq!(a[pos("--disallowedTools") + 1], "WebSearch,WebFetch,Task");
    assert!(a.contains(&s("--verbose")));
}

#[test]
fn claude_argv_ends_with_mcp() {
    let a = claude_argv(&job(), "{\"m\":1}");
    assert_eq!(
        a[a.len() - 3..],
        ["--mcp-config", "{\"m\":1}", "--strict-mcp-config"]
    );
}

#[test]
fn check_env_has_three() {
    assert_eq!(
        check_env("/h", "/c"),
        vec![
            (s("HOME"), s("/h")),
            (s("PATH"), s(SANDBOX_PATH)),
            (s("CARGO_HOME"), s("/c")),
        ]
    );
}

#[test]
fn check_argv_is_sh_c() {
    assert_eq!(check_argv("cargo test"), ["sh", "-c", "cargo test"]);
}

#[test]
fn runnable_cases() {
    assert!(runnable("cargo fmt --all -- --check"));
    assert!(!runnable("CHANGE_BASE=artifacts/main pigeon dev ci-scoped"));
    assert!(!runnable("rustup component add clippy"));
    assert!(runnable("cargo rustup thing"));
}

#[test]
fn sandboxed_prefix() {
    let out = sandboxed(&[(s("A"), s("1")), (s("B"), s("2"))], &[s("echo"), s("hi")]);
    assert_eq!(
        out,
        [
            "sh",
            "-c",
            "umask 000 && exec \"$@\"",
            "sh",
            "unshare",
            "--user",
            "--map-users=100000,0,65536",
            "--map-groups=100000,0,65536",
            "--setuid=1000",
            "--setgid=1000",
            "env",
            "-i",
            "A=1",
            "B=2",
            "echo",
            "hi",
        ]
    );
}

#[test]
fn sandboxed_without_env() {
    let out = sandboxed(&[], &[s("true")]);
    assert_eq!(out[out.len() - 3..], ["env", "-i", "true"]);
}

#[test]
fn mcp_config_args() {
    let v: Value = serde_json::from_str(&mcp_config("/bin/pigeon", "/b.json", "/e.json")).unwrap();
    let p = &v["mcpServers"]["pool"];
    assert_eq!(p["command"], "/bin/pigeon");
    assert_eq!(
        p["args"],
        json!([
            "agent",
            "pool-tools",
            "--brief",
            "/b.json",
            "--escalation",
            "/e.json"
        ])
    );
}

#[test]
fn stream_turns_and_cost() {
    let out =
        "{\"type\":\"system\"}\n{\"type\":\"result\",\"num_turns\":7,\"total_cost_usd\":0.42}\n";
    let st = stream(out);
    assert_eq!(st.turns, 7);
    assert_eq!(st.cost, Some(s("0.42")));
    assert!(!st.error);
    assert_eq!(st.stop, None);
}

#[test]
fn stream_error() {
    let st = stream("{\"type\":\"result\",\"is_error\":true}");
    assert!(st.error);
    assert_eq!(st.turns, 0);
    assert_eq!(st.cost, None);
}

#[test]
fn stream_last_result_wins() {
    let out = "{\"type\":\"result\",\"num_turns\":1}\n{\"type\":\"result\",\"num_turns\":2}\n";
    assert_eq!(stream(out).turns, 2);
}

#[test]
fn stream_cost_must_be_number() {
    let st = stream("{\"type\":\"result\",\"total_cost_usd\":\"0.4\"}");
    assert_eq!(st.cost, None);
}

#[test]
fn stream_turns_too_big() {
    let st = stream("{\"type\":\"result\",\"num_turns\":99999999999}");
    assert_eq!(st.turns, 0);
}

#[test]
fn stream_stop() {
    let out = "{\"type\":\"assistant\",\"text\":\"pool: task b9.1 spent its dollars\\\"}\n";
    assert_eq!(
        stream(out).stop,
        Some(s("pool: task b9.1 spent its dollars"))
    );
}

#[test]
fn stream_stop_to_newline_and_end() {
    assert_eq!(stream("x pool: a b\nrest").stop, Some(s("pool: a b")));
    assert_eq!(stream("pool: tail").stop, Some(s("pool: tail")));
}

#[test]
fn stream_no_result_line() {
    let st = stream("garbage\n{\"type\":\"system\"}\n");
    assert_eq!(st, Stream::default());
}

#[test]
fn stream_stop_without_result() {
    let st = stream("pool: out of minutes\"");
    assert_eq!(st.stop, Some(s("pool: out of minutes")));
    assert_eq!(st.turns, 0);
}

#[test]
fn outcome_branches() {
    let ok = Stream::default();
    assert_eq!(outcome(false, false, &ok, true), (Outcome::Done, s("done")));
    assert_eq!(
        outcome(false, false, &ok, false),
        (Outcome::Failed, s("a check failed"))
    );
    let err = Stream {
        error: true,
        ..Stream::default()
    };
    assert_eq!(
        outcome(false, false, &err, true),
        (Outcome::Failed, s("the harness ended in an error"))
    );
    let stop = Stream {
        stop: Some(s("pool: x")),
        error: true,
        ..Stream::default()
    };
    assert_eq!(
        outcome(false, false, &stop, false),
        (Outcome::Stopped, s("pool: x"))
    );
    assert_eq!(
        outcome(false, true, &stop, false),
        (Outcome::Stopped, s("minutes"))
    );
    assert_eq!(
        outcome(true, false, &ok, true),
        (Outcome::Escalated, s("the model escalated"))
    );
}

#[test]
fn outcome_escalated_beats_timed_out() {
    assert_eq!(
        outcome(true, true, &Stream::default(), true).0,
        Outcome::Escalated
    );
}

#[test]
fn commit_message_done() {
    let m = commit_message(&job(), &[s("a.rs"), s("b.rs")], false);
    assert_eq!(
        m,
        "feat(x): y\n\np\n\nChange-Scope: a.rs\nChange-Scope: b.rs\nPool-Task: b9.1"
    );
}

#[test]
fn commit_message_wip() {
    let m = commit_message(&job(), &[s("a.rs")], true);
    assert_eq!(
        m,
        "wip: feat(x): y\n\np\n\nChange-Scope: a.rs\nPool-Task: b9.1"
    );
}

#[test]
fn commit_message_fallbacks() {
    let mut j = job();
    j.brief = json!({"title": "T", "steps": []});
    assert_eq!(commit_message(&j, &[], false), "T\n\n\n\nPool-Task: b9.1");
    j.brief = json!({"steps": []});
    assert!(commit_message(&j, &[], false).starts_with("pool task\n"));
}

#[test]
fn pooled_serde_skips_empty() {
    let p = Pooled {
        outcome: Outcome::Done,
        why: s("done"),
        turns: 3,
        harness_cost: None,
        checks: vec![],
        escalation: None,
        pushed: None,
    };
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v, json!({"outcome": "done", "why": "done", "turns": 3}));
    let back: Pooled = serde_json::from_value(v).unwrap();
    assert_eq!(back, p);
}

#[test]
fn outcome_serde_kebab() {
    assert_eq!(
        serde_json::to_string(&Outcome::Escalated).unwrap(),
        "\"escalated\""
    );
}

fn rpc(line: &str) -> (Option<Value>, Option<Value>) {
    let (r, e) = answer(line, "BRIEF", "1.2");
    (r.map(|r| serde_json::from_str(&r).unwrap()), e)
}

#[test]
fn answer_parse_error() {
    let (r, e) = rpc("not json");
    let r = r.unwrap();
    assert_eq!(r["error"]["code"], -32700);
    assert_eq!(r["error"]["message"], "parse error");
    assert_eq!(r["id"], Value::Null);
    assert!(e.is_none());
}

#[test]
fn answer_notification() {
    assert_eq!(
        rpc(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        (None, None)
    );
}

#[test]
fn answer_initialize() {
    let (r, _) = rpc(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
    );
    let r = r.unwrap();
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(r["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(
        r["result"]["serverInfo"],
        json!({"name": "pool", "version": "1.2"})
    );
}

#[test]
fn answer_initialize_default_version() {
    let (r, _) = rpc(r#"{"id":1,"method":"initialize"}"#);
    assert_eq!(r.unwrap()["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn answer_tools_list() {
    let (r, _) = rpc(r#"{"id":2,"method":"tools/list"}"#);
    let r = r.unwrap();
    let names: Vec<&str> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["brief_show", "pool_escalate"]);
}

#[test]
fn answer_brief_show() {
    let (r, e) = rpc(r#"{"id":3,"method":"tools/call","params":{"name":"brief_show"}}"#);
    assert_eq!(
        r.unwrap()["result"]["content"][0],
        json!({"type": "text", "text": "BRIEF"})
    );
    assert!(e.is_none());
}

#[test]
fn answer_escalate_ok() {
    let line = r#"{"id":4,"method":"tools/call","params":{"name":"pool_escalate","arguments":{"question":"q","tried":"t","evidence":"ev"}}}"#;
    let (r, e) = rpc(line);
    let r = r.unwrap();
    assert_eq!(r["result"]["content"][0]["text"], ESCALATED);
    assert!(r["result"].get("isError").is_none());
    assert_eq!(
        e,
        Some(json!({"question": "q", "tried": "t", "evidence": "ev"}))
    );
}

#[test]
fn answer_escalate_without_evidence() {
    let line = r#"{"id":4,"method":"tools/call","params":{"name":"pool_escalate","arguments":{"question":"q","tried":"t"}}}"#;
    assert_eq!(
        rpc(line).1,
        Some(json!({"question": "q", "tried": "t", "evidence": ""}))
    );
}

#[test]
fn answer_escalate_missing_tried() {
    let line = r#"{"id":5,"method":"tools/call","params":{"name":"pool_escalate","arguments":{"question":"q"}}}"#;
    let (r, e) = rpc(line);
    let r = r.unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(
        r["result"]["content"][0]["text"],
        "pool_escalate needs `question` and `tried`"
    );
    assert!(e.is_none());
}

#[test]
fn answer_unknown_tool() {
    let (r, e) = rpc(r#"{"id":6,"method":"tools/call","params":{"name":"nope"}}"#);
    let r = r.unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["content"][0]["text"], "no tool `nope`");
    assert!(e.is_none());
}

#[test]
fn answer_unknown_method() {
    let (r, _) = rpc(r#"{"id":7,"method":"resources/list"}"#);
    let r = r.unwrap();
    assert_eq!(r["error"]["code"], -32601);
    assert_eq!(r["error"]["message"], "no method `resources/list`");
}

#[test]
fn answer_echoes_ids() {
    assert_eq!(
        rpc(r#"{"id":42,"method":"tools/list"}"#).0.unwrap()["id"],
        42
    );
    assert_eq!(
        rpc(r#"{"id":"abc","method":"tools/list"}"#).0.unwrap()["id"],
        "abc"
    );
}

#[test]
fn tools_shape() {
    let t = tools();
    assert_eq!(
        t[1]["inputSchema"]["required"],
        json!(["question", "tried"])
    );
    assert_eq!(t[0]["inputSchema"]["properties"], json!({}));
}
