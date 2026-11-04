//! `infra.plan`, `infra.apply` and `serve`; see the phase 3 amendment §6 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;

use backplane::serve::Server;
use backplane::{Error, Instance, Request, run_with};
use serde_json::{Value, json};
use tempfile::TempDir;

const MANIFEST: &str = "description: A test instance.\nschema: v0\nname: test\nrepository: https://example.invalid/test\npaths:\n  policies: .config/policies\n  providers: .config/providers.yaml\n  releases: .config/releases.yaml\n";

const PROOF: &str = "variable \"message\" {\n  type = string\n}\n\nresource \"terraform_data\" \"this\" {\n  input = var.message\n}\n";

const PASSPHRASE: &str = "test-passphrase-0123456789";

const TOKEN: &str = "test-token-0123456789abcdef0123456789";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

/// An instance with one schema-v0 unit of built-in resources.
fn fixture() -> TempDir {
    let root = TempDir::new().expect("tempdir");
    let r = root.path();
    write(r, ".config/instance.yaml", MANIFEST);
    write(r, ".config/policies/.keep", "");
    write(
        r,
        ".config/providers.yaml",
        "description: Test allowlist.\nproviders: {}\n",
    );
    write(r, ".config/releases.yaml", "");
    write(
        r,
        "infra/bootstrap/unit.yaml",
        "schema: 0\nname: bootstrap\nkind: infra\nmodules:\n  - name: proof\n    source: ./modules/proof\n    inputs: { message: hello }\n",
    );
    write(r, "infra/bootstrap/modules/proof/main.tf", PROOF);
    root
}

fn var(k: &str) -> Option<String> {
    (k == "COBOT_STATE_PASSPHRASE").then(|| PASSPHRASE.into())
}

fn request(operation: &str, args: &Value) -> Request {
    serde_json::from_value(json!({ "operation": operation, "args": args })).expect("request")
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn plan_and_apply_need_their_arguments() {
    let root = fixture();
    let instance = Instance::load(root.path()).expect("valid");
    let cases = [
        ("infra.plan", json!({}), "needs argument `state`"),
        (
            "infra.apply",
            json!({ "state": "s" }),
            "needs argument `plan`",
        ),
        (
            "infra.apply",
            json!({ "state": "s", "plan": "x", "units": [] }),
            "takes no argument `units`",
        ),
        (
            "instance.show",
            json!({ "state": "s" }),
            "takes no argument `state`",
        ),
    ];
    for (op, args, want) in cases {
        match run_with(&instance, &request(op, &args), var) {
            Err(Error::Invalid(p)) => assert!(p.iter().any(|m| m.contains(want)), "{op}: {p:?}"),
            other => panic!("{op}: expected Invalid, got {other:?}"),
        }
    }
    let state = TempDir::new().expect("state");
    let bad = run_with(
        &instance,
        &request(
            "infra.plan",
            &json!({ "state": state.path(), "units": "infra/bootstrap" }),
        ),
        var,
    );
    assert!(matches!(bad, Err(Error::Invalid(_))), "{bad:?}");
}

#[test]
fn a_plan_is_applied_by_its_id() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let root = fixture();
    let instance = Instance::load(root.path()).expect("valid");
    let state = TempDir::new().expect("state");
    let planned = run_with(
        &instance,
        &request("infra.plan", &json!({ "state": state.path() })),
        var,
    )
    .expect("plan");
    assert!(planned.ok);
    let id = planned.result["plan"].as_str().expect("id").to_string();
    assert_eq!(
        planned.result["units"][0]["changes"][0],
        json!({ "address": "module.proof.terraform_data.this", "action": "create" })
    );
    assert!(
        !planned.result.to_string().contains("hello"),
        "values never leave"
    );

    let applied = run_with(
        &instance,
        &request("infra.apply", &json!({ "state": state.path(), "plan": id })),
        var,
    )
    .expect("apply");
    assert!(applied.ok);
    assert_eq!(applied.result["units"][0]["unit"], "infra/bootstrap");
    assert!(
        state
            .path()
            .join("infra/bootstrap/terraform.tfstate")
            .exists()
    );

    let again = run_with(
        &instance,
        &request("infra.apply", &json!({ "state": state.path(), "plan": id })),
        var,
    );
    assert!(
        matches!(again, Err(Error::Engine { .. })),
        "applied once: {again:?}"
    );
}

fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
    token: Option<&str>,
) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).expect("connect");
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nhost: test\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .expect("send");
    let _ = s.write_all(body);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("status");
    let json = out.split("\r\n\r\n").nth(1).unwrap_or("null");
    (status, serde_json::from_str(json).expect("json body"))
}

fn http(addr: SocketAddr, method: &str, path: &str, body: &str) -> (u16, Value) {
    send(addr, method, path, body.as_bytes(), Some(TOKEN))
}

fn serving(root: &Path, state: &Path) -> SocketAddr {
    let instance = Instance::load(root).expect("valid");
    let server = Server::bind(
        instance,
        "127.0.0.1:0",
        &state.to_string_lossy(),
        TOKEN,
        false,
    )
    .expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || server.serve(var));
    addr
}

#[test]
fn the_wire_form_round_trips_over_http() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let addr = serving(root.path(), state.path());

    assert_eq!(
        send(addr, "GET", "/healthz", b"", None),
        (200, json!({ "ok": true }))
    );
    assert_eq!(send(addr, "GET", "/v0/operations", b"", None).0, 401);
    assert_eq!(
        send(
            addr,
            "GET",
            "/v0/operations",
            b"",
            Some("wrong-token-0123456789abcdef0123456")
        )
        .0,
        401
    );
    let (status, ops) = http(addr, "GET", "/v0/operations", "");
    assert_eq!(status, 200);
    let names: Vec<&str> = ops
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|o| o["name"].as_str())
        .collect();
    assert!(
        names.contains(&"infra.plan") && names.contains(&"infra.apply"),
        "{names:?}"
    );

    let (status, shown) = http(
        addr,
        "POST",
        "/v0/run",
        r#"{"operation":"instance.show","args":{}}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(shown["operation"], "instance.show");
    assert_eq!(shown["result"]["name"], "test");
    let local: backplane::Response = run_with(
        &Instance::load(root.path()).expect("valid"),
        &request("instance.show", &json!({})),
        var,
    )
    .expect("local");
    assert_eq!(shown, serde_json::to_value(local).expect("json"));

    let (status, refused) = http(
        addr,
        "POST",
        "/v0/run",
        r#"{"operation":"infra.apply","args":{}}"#,
    );
    assert_eq!(status, 400);
    assert!(
        refused["problems"]
            .to_string()
            .contains("needs argument `plan`"),
        "{refused}"
    );
    let (status, refused) = http(
        addr,
        "POST",
        "/v0/run",
        r#"{"operation":"infra.plan","args":{"state":"/elsewhere"}}"#,
    );
    assert_eq!(status, 400, "a request never names the state: {refused}");
    let (status, own) = http(
        addr,
        "POST",
        "/v0/run",
        &json!({ "operation": "infra.apply", "args": { "state": state.path(), "plan": "0".repeat(32) } }).to_string(),
    );
    assert_eq!(
        status, 500,
        "naming the server's own state is accepted: {own}"
    );
    assert!(own["error"].to_string().contains("no stored plan"), "{own}");
    assert_eq!(http(addr, "POST", "/v0/run", "not json").0, 400);
    let mut huge = vec![b'a'; 1024 * 1024 + 1];
    huge.push(0xff);
    assert_eq!(send(addr, "POST", "/v0/run", &huge, Some(TOKEN)).0, 413);
    assert_eq!(http(addr, "GET", "/v0/run", "").0, 405);
    assert_eq!(http(addr, "GET", "/nope", "").0, 404);
}

#[test]
fn a_short_token_is_refused() {
    let root = fixture();
    let instance = Instance::load(root.path()).expect("valid");
    assert!(matches!(
        Server::bind(instance, "127.0.0.1:0", "/state", "short", false),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn only_loopback_binds_without_network() {
    let root = fixture();
    let instance = || Instance::load(root.path()).expect("valid");
    let err = Server::bind(instance(), "0.0.0.0:0", "/state", TOKEN, false);
    assert!(
        matches!(&err, Err(Error::Invalid(p)) if p[0].contains("--network")),
        "{:?}",
        err.as_ref().err()
    );
    assert!(Server::bind(instance(), "0.0.0.0:0", "/state", TOKEN, true).is_ok());
    assert!(Server::bind(instance(), "localhost:0", "/state", TOKEN, false).is_ok());
}

#[test]
fn an_empty_selection_plans_nothing() {
    let root = fixture();
    fs::remove_dir_all(root.path().join("infra")).expect("no units");
    let instance = Instance::load(root.path()).expect("valid");
    let state = TempDir::new().expect("state");
    let err = run_with(
        &instance,
        &request("infra.plan", &json!({ "state": state.path() })),
        var,
    )
    .expect_err("nothing to plan");
    assert!(err.to_string().contains("no units to plan"), "{err}");
    assert!(!state.path().join("plans").exists(), "no plan stored");
}

#[test]
fn health_answers_while_a_plan_runs() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let root = fixture();
    let state = TempDir::new().expect("state");
    let addr = serving(root.path(), state.path());
    let planning = std::thread::spawn(move || {
        http(
            addr,
            "POST",
            "/v0/run",
            r#"{"operation":"infra.plan","args":{}}"#,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    let started = std::time::Instant::now();
    assert_eq!(send(addr, "GET", "/healthz", b"", None).0, 200);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "healthz waited {:?} behind the plan",
        started.elapsed()
    );
    let (status, planned) = planning.join().expect("plan thread");
    assert_eq!(status, 200, "{planned}");
    assert!(planned["result"]["plan"].is_string(), "{planned}");
}

#[test]
fn stalled_requests_leave_the_server_answering() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let addr = serving(root.path(), state.path());
    let stall = || {
        let mut s = TcpStream::connect(addr).expect("connect");
        write!(
            s,
            "POST /v0/run HTTP/1.1\r\nhost: test\r\nauthorization: Bearer {TOKEN}\r\ncontent-length: 500000\r\n\r\n"
        )
        .expect("send");
        s
    };
    let stalled: Vec<TcpStream> = (1..backplane::serve::WORKERS).map(|_| stall()).collect();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        send(addr, "GET", "/healthz", b"", None).0,
        200,
        "the last free worker still answers"
    );
    drop(stalled);
    std::thread::sleep(std::time::Duration::from_millis(500));
    for _ in 0..backplane::serve::WORKERS {
        assert_eq!(
            send(addr, "GET", "/healthz", b"", None).0,
            200,
            "workers are released"
        );
    }
}
