//! `infra.plan`, `infra.apply` and `serve`; see the phase 3 amendment §6 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use backplane::serve::{
    NONCE_HEADER, SIGNATURE_HEADER, Server, TIMESTAMP_HEADER, Tls, WORKERS, sign_request,
    sign_response,
};
use backplane::{Error, Instance, Request, run_with};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
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

    let tfstate = state.path().join("infra/bootstrap/terraform.tfstate");
    let after = fs::read(&tfstate).expect("applied");
    let again = run_with(
        &instance,
        &request("infra.apply", &json!({ "state": state.path(), "plan": id })),
        var,
    )
    .expect("a completed plan reports its recorded result");
    assert!(
        again.result["units"][0]["note"]
            .as_str()
            .is_some_and(|n| n.contains("Applied from this plan earlier.")),
        "{again:?}"
    );
    assert_eq!(
        fs::read(&tfstate).expect("state"),
        after,
        "nothing applied twice"
    );
}

/// A CA, as PEM, and its issuer.
fn ca(name: &str) -> (String, Issuer<'static, KeyPair>) {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, name);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let key = KeyPair::generate().expect("key");
    let cert = params.self_signed(&key).expect("ca");
    (cert.pem(), Issuer::new(params, key))
}

/// A certificate for `name` that `issuer` signed, and its key, as PEM.
fn leaf(
    issuer: &Issuer<'_, KeyPair>,
    name: &str,
    usage: ExtendedKeyUsagePurpose,
) -> (String, String) {
    let mut params = CertificateParams::new(vec![name.to_string()]).expect("params");
    params.extended_key_usages = vec![usage];
    let key = KeyPair::generate().expect("key");
    let cert = params.signed_by(&key, issuer).expect("leaf");
    (cert.pem(), key.serialize_pem())
}

/// The server's TLS material, and clients with and without a certificate it trusts.
struct Pki {
    server: Tls,
    client: Arc<ClientConfig>,
    stranger: Arc<ClientConfig>,
    anonymous: Arc<ClientConfig>,
}

fn pki() -> Pki {
    let (ca_pem, issuer) = ca("instance-ca");
    let (_, other) = ca("other-ca");
    let (cert, key) = leaf(&issuer, "localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(ca_pem.as_bytes()).expect("ca der"))
        .expect("root");
    let roots = Arc::new(roots);
    let builder = || {
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("tls13")
            .with_root_certificates(Arc::clone(&roots))
    };
    let with_cert = |(cert, key): (String, String)| {
        let chain = vec![CertificateDer::from_pem_slice(cert.as_bytes()).expect("cert")];
        let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).expect("key");
        Arc::new(builder().with_client_auth_cert(chain, key).expect("client"))
    };
    Pki {
        server: Tls {
            cert,
            key,
            client_ca: ca_pem,
        },
        client: with_cert(leaf(&issuer, "worker", ExtendedKeyUsagePurpose::ClientAuth)),
        stranger: with_cert(leaf(&other, "worker", ExtendedKeyUsagePurpose::ClientAuth)),
        anonymous: Arc::new(builder().with_no_client_auth()),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// A nonce no test has used.
fn fresh_nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:016x}{:016x}",
        unix_now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A request carrying `extra` header lines.
fn raw_request(method: &str, path: &str, body: &[u8], extra: &str) -> Vec<u8> {
    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\n{extra}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    raw
}

/// The header lines of a request signed under `token`.
fn signature_headers(
    token: &str,
    method: &str,
    path: &str,
    body: &[u8],
    timestamp: u64,
    nonce: &str,
) -> String {
    let sig = sign_request(token, method, path, timestamp, nonce, body);
    format!(
        "{TIMESTAMP_HEADER}: {timestamp}\r\n{NONCE_HEADER}: {nonce}\r\n{SIGNATURE_HEADER}: {sig}\r\n"
    )
}

/// A response's status, its JSON body's text, and its signature header, if any.
fn parse(out: &str) -> (u16, String, Option<String>) {
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((out, "null"));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status in {out:?}"));
    let signature = head.lines().find_map(|l| {
        l.split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case(SIGNATURE_HEADER))
            .map(|(_, v)| v.trim().to_string())
    });
    (status, body.to_string(), signature)
}

/// A server's address, reached in plain HTTP or with a TLS client config, by bearer token or
/// by signed requests.
#[derive(Clone)]
struct Client {
    addr: SocketAddr,
    tls: Option<Arc<ClientConfig>>,
    signed: bool,
}

impl Client {
    fn with(&self, tls: &Arc<ClientConfig>) -> Self {
        Self {
            tls: Some(Arc::clone(tls)),
            ..self.clone()
        }
    }

    /// Everything the server answers to `raw`, until it closes or fails.
    fn exchange(&self, raw: &[u8]) -> String {
        let tcp = TcpStream::connect(self.addr).expect("connect");
        tcp.set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut drain = |s: &mut dyn Read| {
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&chunk[..n]);
            }
        };
        match &self.tls {
            None => {
                let mut s = tcp;
                let _ = s.write_all(raw);
                drain(&mut s);
            }
            Some(config) => {
                let name = ServerName::try_from("localhost").expect("name");
                let conn = ClientConnection::new(Arc::clone(config), name).expect("client");
                let mut s = StreamOwned::new(conn, tcp);
                let _ = s.write_all(raw).and_then(|()| s.flush());
                drain(&mut s);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Sends with `token` as the bearer or, for a signed client, as the signing key; a signed
    /// response must verify under the server's token.
    fn send(&self, method: &str, path: &str, body: &[u8], token: Option<&str>) -> (u16, Value) {
        let nonce = fresh_nonce();
        let extra = match (token, self.signed) {
            (None, _) => String::new(),
            (Some(t), false) => format!("authorization: Bearer {t}\r\n"),
            (Some(t), true) => signature_headers(t, method, path, body, unix_now(), &nonce),
        };
        let out = self.exchange(&raw_request(method, path, body, &extra));
        let (status, text, signature) = parse(&out);
        if let Some(sig) = signature {
            assert_eq!(
                sig,
                sign_response(TOKEN, &nonce, status, text.as_bytes()),
                "the response verifies"
            );
        }
        (status, serde_json::from_str(&text).expect("json body"))
    }

    fn http(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        self.send(method, path, body.as_bytes(), Some(TOKEN))
    }

    /// Whether `/healthz` gets no HTTP answer at all.
    fn refused(&self) -> bool {
        !self
            .exchange(b"GET /healthz HTTP/1.1\r\nhost: test\r\n\r\n")
            .starts_with("HTTP/")
    }
}

fn start(server: Server, tls: Option<&Pki>, signed: bool) -> Client {
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || server.serve(var));
    Client {
        addr,
        tls: tls.map(|p| Arc::clone(&p.client)),
        signed,
    }
}

fn serving_as(root: &Path, state: &Path, tls: Option<&Pki>, signed: bool) -> Client {
    let instance = Instance::load(root).expect("valid");
    let server = Server::bind(
        instance,
        "127.0.0.1:0",
        &state.to_string_lossy(),
        TOKEN,
        false,
        tls.map(|p| &p.server),
        signed,
    )
    .expect("bind");
    start(server, tls, signed)
}

fn serving(root: &Path, state: &Path, tls: Option<&Pki>) -> Client {
    serving_as(root, state, tls, false)
}

fn the_wire_form_round_trips(tls: Option<&Pki>, signed: bool) {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let c = serving_as(root.path(), state.path(), tls, signed);

    assert_eq!(
        c.send("GET", "/healthz", b"", None),
        (200, json!({ "ok": true }))
    );
    assert_eq!(c.send("GET", "/v0/operations", b"", None).0, 401);
    assert_eq!(
        c.send(
            "GET",
            "/v0/operations",
            b"",
            Some("wrong-token-0123456789abcdef0123456")
        )
        .0,
        401
    );
    let (status, ops) = c.http("GET", "/v0/operations", "");
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

    let (status, shown) = c.http(
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

    let (status, refused) = c.http(
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
    let (status, refused) = c.http(
        "POST",
        "/v0/run",
        r#"{"operation":"infra.plan","args":{"state":"/elsewhere"}}"#,
    );
    assert_eq!(status, 400, "a request never names the state: {refused}");
    let (status, own) = c.http(
        "POST",
        "/v0/run",
        &json!({ "operation": "infra.apply", "args": { "state": state.path(), "plan": "0".repeat(32) } }).to_string(),
    );
    assert_eq!(
        status, 500,
        "naming the server's own state is accepted: {own}"
    );
    assert!(own["error"].to_string().contains("no stored plan"), "{own}");
    assert_eq!(c.http("POST", "/v0/run", "not json").0, 400);
    let mut huge = vec![b'a'; 1024 * 1024 + 1];
    huge.push(0xff);
    assert_eq!(c.send("POST", "/v0/run", &huge, Some(TOKEN)).0, 413);
    assert_eq!(c.http("GET", "/v0/run", "").0, 405);
    assert_eq!(c.http("GET", "/nope", "").0, 404);
}

#[test]
fn the_wire_form_round_trips_over_http() {
    the_wire_form_round_trips(None, false);
}

#[test]
fn the_wire_form_round_trips_over_mutual_tls() {
    the_wire_form_round_trips(Some(&pki()), false);
}

#[test]
fn the_wire_form_round_trips_signed() {
    the_wire_form_round_trips(None, true);
}

#[test]
fn signed_mode_verifies_every_request_and_signs_every_answer() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let c = serving_as(root.path(), state.path(), None, true);
    let ops = |extra: &str| parse(&c.exchange(&raw_request("GET", "/v0/operations", b"", extra)));

    let nonce = fresh_nonce();
    let good = signature_headers(TOKEN, "GET", "/v0/operations", b"", unix_now(), &nonce);
    let (status, text, sig) = ops(&good);
    assert_eq!(status, 200, "a signed request needs no bearer header");
    assert_eq!(
        sig.as_deref(),
        Some(sign_response(TOKEN, &nonce, 200, text.as_bytes()).as_str()),
        "the answer is signed"
    );
    assert_eq!(ops(&good).0, 401, "a nonce is spent once");

    assert_eq!(
        ops(&format!("authorization: Bearer {TOKEN}\r\n")).0,
        401,
        "an unsigned request is refused, bearer or not"
    );
    let at = |t: u64| signature_headers(TOKEN, "GET", "/v0/operations", b"", t, &fresh_nonce());
    assert_eq!(ops(&at(unix_now() - 120)).0, 401, "a stale timestamp");
    assert_eq!(ops(&at(unix_now() + 120)).0, 401, "a future timestamp");
    let other_path = signature_headers(TOKEN, "GET", "/v0/run", b"", unix_now(), &fresh_nonce());
    assert_eq!(ops(&other_path).0, 401, "a signature over another request");
    let wrong_key = signature_headers(
        "another-token-0123456789abcdef0123",
        "GET",
        "/v0/operations",
        b"",
        unix_now(),
        &fresh_nonce(),
    );
    let (status, _, sig) = ops(&wrong_key);
    assert_eq!(status, 401, "a bad signature");
    assert!(sig.is_none(), "a refusal is not signed");

    let body = br#"{"operation":"instance.show","args":{}}"#;
    let signed_for_other_body =
        signature_headers(TOKEN, "POST", "/v0/run", b"{}", unix_now(), &fresh_nonce());
    let (status, ..) = parse(&c.exchange(&raw_request(
        "POST",
        "/v0/run",
        body,
        &signed_for_other_body,
    )));
    assert_eq!(status, 401, "a tampered body");
}

#[test]
fn a_response_signature_covers_status_and_body() {
    let nonce = fresh_nonce();
    let sig = sign_response(TOKEN, &nonce, 409, b"{}");
    assert_ne!(
        sig,
        sign_response(TOKEN, &nonce, 200, b"{}"),
        "a changed status"
    );
    assert_ne!(
        sig,
        sign_response(TOKEN, &nonce, 409, b"[]"),
        "a changed body"
    );
    assert_ne!(
        sig,
        sign_response(TOKEN, &fresh_nonce(), 409, b"{}"),
        "another request"
    );
}

#[test]
fn mutual_tls_refuses_clients_without_a_trusted_certificate() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let pki = pki();
    let c = serving(root.path(), state.path(), Some(&pki));
    assert_eq!(c.send("GET", "/healthz", b"", None).0, 200);
    assert!(c.with(&pki.anonymous).refused(), "no client certificate");
    assert!(c.with(&pki.stranger).refused(), "another CA's certificate");
    assert!(
        Client {
            tls: None,
            ..c.clone()
        }
        .refused(),
        "plain HTTP"
    );
    assert_eq!(
        c.send("GET", "/v0/operations", b"", None).0,
        401,
        "a certificate does not replace the token"
    );
}

#[test]
fn a_short_token_is_refused() {
    let root = fixture();
    let instance = Instance::load(root.path()).expect("valid");
    assert!(matches!(
        Server::bind(
            instance,
            "127.0.0.1:0",
            "/state",
            "short",
            false,
            None,
            false
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn only_loopback_binds_without_network_and_network_needs_tls_or_signing() {
    let root = fixture();
    let instance = || Instance::load(root.path()).expect("valid");
    let pki = pki();
    let err = Server::bind(instance(), "0.0.0.0:0", "/state", TOKEN, false, None, false);
    assert!(
        matches!(&err, Err(Error::Invalid(p)) if p[0].contains("--network")),
        "{:?}",
        err.as_ref().err()
    );
    let err = Server::bind(instance(), "0.0.0.0:0", "/state", TOKEN, true, None, false);
    assert!(
        matches!(&err, Err(Error::Invalid(p)) if p[0].contains("BACKPLANE_TLS_CERT") && p[0].contains("--signed")),
        "{:?}",
        err.as_ref().err()
    );
    assert!(
        Server::bind(instance(), "0.0.0.0:0", "/state", TOKEN, true, None, true).is_ok(),
        "--network with --signed"
    );
    assert!(
        Server::bind(
            instance(),
            "0.0.0.0:0",
            "/state",
            TOKEN,
            true,
            Some(&pki.server),
            false
        )
        .is_ok()
    );
    assert!(
        Server::bind(
            instance(),
            "localhost:0",
            "/state",
            TOKEN,
            false,
            None,
            false
        )
        .is_ok()
    );
}

#[test]
fn tls_material_is_all_or_nothing_and_must_parse() {
    let pki = pki();
    let vars = |set: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            set.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        }
    };
    assert!(matches!(Tls::from_vars(vars(&[])), Ok(None)));
    assert!(matches!(
        Tls::from_vars(vars(&[("BACKPLANE_TLS_CERT", "x")])),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Tls::from_vars(vars(&[
            ("BACKPLANE_TLS_CERT", "a"),
            ("BACKPLANE_TLS_KEY", "b"),
            ("BACKPLANE_TLS_CLIENT_CA", "c"),
        ])),
        Ok(Some(_))
    ));
    let root = fixture();
    let bad = Tls {
        key: "not a key".into(),
        ..pki.server.clone()
    };
    let err = Server::bind(
        Instance::load(root.path()).expect("valid"),
        "127.0.0.1:0",
        "/state",
        TOKEN,
        false,
        Some(&bad),
        false,
    );
    match err {
        Err(Error::Invalid(p)) => {
            assert!(p[0].starts_with("BACKPLANE_TLS_KEY"), "{p:?}");
            assert!(!p[0].contains("not a key"), "the key never appears: {p:?}");
        }
        other => panic!("expected Invalid, got {:?}", other.err()),
    }
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
    let pki = pki();
    let c = serving(root.path(), state.path(), Some(&pki));
    let other = c.clone();
    let planning = std::thread::spawn(move || {
        other.http("POST", "/v0/run", r#"{"operation":"infra.plan","args":{}}"#)
    });
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    assert_eq!(c.send("GET", "/healthz", b"", None).0, 200);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "healthz waited {:?} behind the plan",
        started.elapsed()
    );
    let (status, busy) = c.http("POST", "/v0/run", r#"{"operation":"infra.plan","args":{}}"#);
    assert_eq!(
        status, 409,
        "a second plan is turned away, not queued: {busy}"
    );
    let (status, invalid) = c.http(
        "POST",
        "/v0/run",
        r#"{"operation":"infra.apply","args":{}}"#,
    );
    assert_eq!(
        status, 400,
        "an invalid request is invalid even while busy: {invalid}"
    );
    let (status, planned) = planning.join().expect("plan thread");
    assert_eq!(status, 200, "{planned}");
    assert!(planned["result"]["plan"].is_string(), "{planned}");
}

#[test]
fn stalled_requests_leave_the_server_answering() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let c = serving(root.path(), state.path(), None);
    let stall = || {
        let mut s = TcpStream::connect(c.addr).expect("connect");
        write!(
            s,
            "POST /v0/run HTTP/1.1\r\nhost: test\r\nauthorization: Bearer {TOKEN}\r\ncontent-length: 500000\r\n\r\n"
        )
        .expect("send");
        s
    };
    let stalled: Vec<TcpStream> = (1..WORKERS).map(|_| stall()).collect();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        c.send("GET", "/healthz", b"", None).0,
        200,
        "the last free worker still answers"
    );
    drop(stalled);
    std::thread::sleep(Duration::from_millis(500));
    for _ in 0..WORKERS {
        assert_eq!(
            c.send("GET", "/healthz", b"", None).0,
            200,
            "workers are released"
        );
    }
}

#[test]
fn the_deadline_releases_every_stalled_worker() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let pki = pki();
    let bind = |tls: Option<&Pki>| {
        let server = Server::bind(
            Instance::load(root.path()).expect("valid"),
            "127.0.0.1:0",
            &state.path().to_string_lossy(),
            TOKEN,
            false,
            tls.map(|p| &p.server),
            false,
        )
        .expect("bind")
        .with_deadline(Duration::from_secs(1));
        start(server, tls, false)
    };
    let plain = bind(None);
    let mutual = bind(Some(&pki));
    let body_stalls: Vec<TcpStream> = (0..WORKERS)
        .map(|_| {
            let mut s = TcpStream::connect(plain.addr).expect("connect");
            write!(
                s,
                "POST /v0/run HTTP/1.1\r\nhost: test\r\nauthorization: Bearer {TOKEN}\r\ncontent-length: 500000\r\n\r\n{{"
            )
            .expect("send");
            s
        })
        .collect();
    let handshake_stalls: Vec<TcpStream> = (0..WORKERS)
        .map(|_| TcpStream::connect(mutual.addr).expect("connect"))
        .collect();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        plain.send("GET", "/healthz", b"", None).0,
        200,
        "a stalled body"
    );
    assert_eq!(
        mutual.send("GET", "/healthz", b"", None).0,
        200,
        "a stalled handshake"
    );
    drop((body_stalls, handshake_stalls));
}

#[test]
fn request_framing_is_strict() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let c = serving(root.path(), state.path(), None);
    let status = |raw: &str| -> String {
        c.exchange(raw.as_bytes())
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string()
    };
    let auth = format!("authorization: Bearer {TOKEN}\r\n");
    assert_eq!(
        status(&format!(
            "POST /v0/run HTTP/1.1\r\n{auth}content-length: 2\r\ncontent-length: 40\r\n\r\n{{}}"
        )),
        "400",
        "two content-lengths"
    );
    assert_eq!(
        status(&format!(
            "POST /v0/run HTTP/1.1\r\n{auth}expect: something\r\ncontent-length: 2\r\n\r\n{{}}"
        )),
        "417"
    );
    let big = "a".repeat(17 * 1024);
    assert_eq!(
        status(&format!("GET /healthz HTTP/1.1\r\nx-big: {big}\r\n\r\n")),
        "431",
        "headers over 16 KiB"
    );

    let body = r#"{"operation":"instance.show","args":{}}"#;
    let mut s = TcpStream::connect(c.addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write!(
        s,
        "POST /v0/run HTTP/1.1\r\n{auth}expect: 100-continue\r\ncontent-length: {}\r\n\r\n",
        body.len()
    )
    .expect("head");
    let mut interim = [0u8; 25];
    s.read_exact(&mut interim).expect("100 Continue");
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    s.write_all(body.as_bytes()).expect("body");
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 200 "), "{out}");
}

#[test]
fn a_stop_drains_requests_in_hand_then_returns() {
    let root = fixture();
    let state = TempDir::new().expect("state");
    let server = Server::bind(
        Instance::load(root.path()).expect("valid"),
        "127.0.0.1:0",
        &state.path().to_string_lossy(),
        TOKEN,
        false,
        None,
        false,
    )
    .expect("bind");
    let addr = server.local_addr().expect("addr");
    let stop = server.stopper();
    let serving = std::thread::spawn(move || server.serve(var));

    let half_sent = |body: &[u8]| {
        let mut s = TcpStream::connect(addr).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        write!(
            s,
            "POST /v0/run HTTP/1.1\r\nhost: test\r\nauthorization: Bearer {TOKEN}\r\ncontent-length: {}\r\n\r\n",
            body.len()
        )
        .expect("head");
        s.write_all(&body[..5]).expect("half the body");
        s
    };
    let body = br#"{"operation":"instance.show","args":{}}"#;
    let apply =
        br#"{"operation":"infra.apply","args":{"plan":"00000000000000000000000000000000"}}"#;
    let mut s = half_sent(body);
    let mut late_apply = half_sent(apply);
    std::thread::sleep(Duration::from_millis(300));

    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!serving.is_finished(), "a request in hand holds the drain");
    s.write_all(&body[5..]).expect("the rest");
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(
        out.starts_with("HTTP/1.1 200 "),
        "answered while draining: {out}"
    );
    late_apply.write_all(&apply[5..]).expect("the rest");
    let mut out = String::new();
    let _ = late_apply.read_to_string(&mut out);
    assert!(
        out.starts_with("HTTP/1.1 503 "),
        "no apply starts while draining: {out}"
    );

    let started = Instant::now();
    while !serving.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "serve returns once drained"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    serving.join().expect("serve");
    let late = TcpStream::connect(addr).map(|mut s| {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = s.write_all(b"GET /healthz HTTP/1.1\r\nhost: test\r\n\r\n");
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    });
    assert!(
        late.map_or(true, |out| !out.starts_with("HTTP/")),
        "nothing is accepted after the stop"
    );
}
