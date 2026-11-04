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

fn http(addr: SocketAddr, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).expect("connect");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nhost: test\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("send");
    let mut out = String::new();
    s.read_to_string(&mut out).expect("read");
    let status = out
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("status");
    let json = out.split("\r\n\r\n").nth(1).unwrap_or("null");
    (status, serde_json::from_str(json).expect("json body"))
}

#[test]
fn the_wire_form_round_trips_over_http() {
    let root = fixture();
    let instance = Instance::load(root.path()).expect("valid");
    let server = Server::bind(instance, "127.0.0.1:0").expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || server.serve(var));

    assert_eq!(
        http(addr, "GET", "/healthz", ""),
        (200, json!({ "ok": true }))
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
        r#"{"operation":"infra.apply","args":{"state":"s"}}"#,
    );
    assert_eq!(status, 400);
    assert!(
        refused["problems"]
            .to_string()
            .contains("needs argument `plan`"),
        "{refused}"
    );
    assert_eq!(http(addr, "POST", "/v0/run", "not json").0, 400);
    assert_eq!(http(addr, "GET", "/v0/run", "").0, 405);
    assert_eq!(http(addr, "GET", "/nope", "").0, 404);
}
