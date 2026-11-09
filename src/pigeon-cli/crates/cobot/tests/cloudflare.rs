//! A run's own Cloudflare token against a fake API; see
//! `.claude/decisions/2026-11-05-development-pigeon-its-own-instance.md` §4.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use cobot::cloudflare::Api;
use serde_json::{Value, json};

const TOKEN: &str = "run-token-0123456789";

/// Answers each `GET` with `route(path)`'s result, or 404; every request must carry `TOKEN`.
fn serve(route: fn(&str) -> Option<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let target = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let mut authorized = false;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).expect("header");
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':')
                    && k.eq_ignore_ascii_case("authorization")
                {
                    authorized = v.trim() == format!("Bearer {TOKEN}");
                }
            }
            let (status, body) = match (authorized, route(&target)) {
                (false, _) => (403, json!({ "success": false })),
                (true, Some(result)) => (200, json!({ "success": true, "result": result })),
                (true, None) => (404, json!({ "success": false })),
            };
            let text = body.to_string();
            let mut out = stream;
            let _ = write!(
                out,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    format!("http://{addr}")
}

fn one_account(path: &str) -> Option<Value> {
    match path {
        "/accounts?per_page=2" => Some(json!([{ "id": "acct-1" }])),
        "/zones?name=a.com&per_page=2" => Some(json!([{ "id": "zone-a", "name": "a.com" }])),
        "/zones?name=b.org&per_page=2" => Some(json!([])),
        "/accounts/acct-1/tokens/verify" => Some(json!({ "id": "tok-1", "status": "active" })),
        _ => None,
    }
}

fn two_accounts(path: &str) -> Option<Value> {
    (path == "/accounts?per_page=2").then(|| json!([{ "id": "a" }, { "id": "b" }]))
}

#[test]
fn the_token_resolves_its_account_zones_and_own_id() {
    let api = Api::new(Some(&serve(one_account)), TOKEN.into());
    assert_eq!(api.account().as_deref(), Ok("acct-1"));
    assert_eq!(api.zone_id("a.com").as_deref(), Ok("zone-a"));
    assert_eq!(
        api.zone_id("b.org"),
        Err("the token does not see zone `b.org`".into())
    );
    assert_eq!(
        api.token_id("acct-1").as_deref(),
        Ok("tok-1"),
        "an account token verifies under its account"
    );
}

#[test]
fn a_token_seeing_more_than_one_account_is_refused() {
    let api = Api::new(Some(&serve(two_accounts)), TOKEN.into());
    assert_eq!(
        api.account(),
        Err("the token sees more than one account; scope it to one".into())
    );
}

#[test]
fn errors_name_the_call_never_the_token() {
    let base = serve(one_account);
    let wrong = Api::new(Some(&base), "wrong-token-9876543210".into());
    let err = wrong.account().expect_err("refused");
    assert_eq!(err, "GET /accounts: status 403");
    assert!(!format!("{wrong:?}").contains("wrong-token"));
    let err = wrong.token_id("acct-1").expect_err("refused");
    assert!(!err.contains("wrong-token"), "{err}");
}

#[test]
fn a_context_never_prints_its_secrets() {
    let ctx = cobot::Context {
        root: "/r".into(),
        providers: "/r/.config".into(),
        state: "/s".into(),
        state_passphrase: "state-pass-secret".into(),
        secrets_passphrase: Some("secrets-pass-secret".into()),
        workload_secrets_passphrase: None,
        today: 0,
        credentials: Some(r#"{"cloudflare":{"api_token":"runner-secret"}}"#.into()),
        cloudflare_token: Some("token-secret".into()),
        cloudflare_api: None,
        platform: None,
        adopt: false,
        provider_config: None,
    };
    let shown = format!("{ctx:?}");
    for secret in [
        "state-pass",
        "secrets-pass",
        "runner-secret",
        "token-secret",
    ] {
        assert!(!shown.contains(secret), "{shown}");
    }
    assert!(shown.contains("cloudflare_token: \"set\""), "{shown}");
}
