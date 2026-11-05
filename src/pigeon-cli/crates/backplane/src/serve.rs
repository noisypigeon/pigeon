//! `pigeon backplane serve`: the wire form over HTTP, in mutual TLS or signed mode beyond
//! loopback; see the phase 3 amendment §6, the "Worker–Container mutual TLS" amendment and the
//! "Phase 3 spike report" amendment §2 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use serde_json::{Value, json};

use cobot::link::constant_eq;

use crate::{Effect, Error, Instance, Request, catalog, run_with};

/// The largest request body read: 1 MiB.
const MAX_BODY: u64 = 1024 * 1024;

/// The largest request line and headers read: 16 KiB.
const MAX_HEAD: usize = 16 * 1024;

/// The most headers a request may carry.
const MAX_HEADERS: usize = 32;

/// The longest one read or write waits.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest a connection takes from accept to its whole request read, handshake included.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// The environment variable holding the bearer token every `/v0/` request must carry.
pub const TOKEN_VAR: &str = "BACKPLANE_TOKEN";

/// The environment variable holding the server's certificate chain, as PEM.
pub const TLS_CERT_VAR: &str = "BACKPLANE_TLS_CERT";

/// The environment variable holding the server's private key, as PEM.
pub const TLS_KEY_VAR: &str = "BACKPLANE_TLS_KEY";

/// The environment variable holding the CA that issues client certificates, as PEM.
pub const TLS_CLIENT_CA_VAR: &str = "BACKPLANE_TLS_CLIENT_CA";

pub use cobot::link::{
    NONCE_HEADER, SIGNATURE_HEADER, SKEW, TIMESTAMP_HEADER, sign_request, sign_response,
};

/// How long an idle worker waits before checking for a connection or a stop again.
const POLL: Duration = Duration::from_millis(50);

/// The shortest token accepted.
const MIN_TOKEN: usize = 32;

/// The workers answering connections; more connections wait for one.
pub const WORKERS: usize = 16;

/// The server's TLS material, as PEM: every client must present a certificate `client_ca` issued.
#[derive(Clone)]
pub struct Tls {
    /// The server's certificate chain.
    pub cert: String,
    /// The server's private key.
    pub key: String,
    /// The CA that issues client certificates.
    pub client_ca: String,
}

impl Tls {
    /// The material from [`TLS_CERT_VAR`], [`TLS_KEY_VAR`] and [`TLS_CLIENT_CA_VAR`], read
    /// through `var`; `None` when none is set.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when only some are set.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, Error> {
        let get = |k: &str| var(k).filter(|v| !v.trim().is_empty());
        match (get(TLS_CERT_VAR), get(TLS_KEY_VAR), get(TLS_CLIENT_CA_VAR)) {
            (Some(cert), Some(key), Some(client_ca)) => Ok(Some(Self {
                cert,
                key,
                client_ca,
            })),
            (None, None, None) => Ok(None),
            _ => Err(Error::Invalid(vec![format!(
                "set all of {TLS_CERT_VAR}, {TLS_KEY_VAR} and {TLS_CLIENT_CA_VAR}, or none"
            )])),
        }
    }

    /// TLS 1.3 only, requiring a client certificate.
    fn config(&self) -> Result<Arc<ServerConfig>, Error> {
        let invalid = |var: &str, why: String| Error::Invalid(vec![format!("{var}: {why}")]);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let certs = CertificateDer::pem_slice_iter(self.cert.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| invalid(TLS_CERT_VAR, e.to_string()))?;
        if certs.is_empty() {
            return Err(invalid(TLS_CERT_VAR, "no certificate".into()));
        }
        let key = PrivateKeyDer::from_pem_slice(self.key.as_bytes())
            .map_err(|e| invalid(TLS_KEY_VAR, e.to_string()))?;
        let mut roots = RootCertStore::empty();
        for ca in CertificateDer::pem_slice_iter(self.client_ca.as_bytes()) {
            let ca = ca.map_err(|e| invalid(TLS_CLIENT_CA_VAR, e.to_string()))?;
            roots
                .add(ca)
                .map_err(|e| invalid(TLS_CLIENT_CA_VAR, e.to_string()))?;
        }
        if roots.is_empty() {
            return Err(invalid(TLS_CLIENT_CA_VAR, "no certificate".into()));
        }
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
                .build()
                .map_err(|e| invalid(TLS_CLIENT_CA_VAR, e.to_string()))?;
        let config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| invalid(TLS_CERT_VAR, e.to_string()))?
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
            .map_err(|e| invalid(TLS_KEY_VAR, e.to_string()))?;
        Ok(Arc::new(config))
    }
}

/// A bound server answering one instance's operations against one state.
pub struct Server {
    listener: TcpListener,
    tls: Option<Arc<ServerConfig>>,
    deadline: Duration,
    instance: Instance,
    state: String,
    token: String,
    /// Whether `/v0/` routes take signed requests instead of the bearer token.
    signed: bool,
    /// Nonces of verified requests, by the time they were seen; pruned past twice [`SKEW`].
    seen: Mutex<HashMap<String, u64>>,
    /// Held by `plan` or `apply`, which share the state's mirror; a second gets 409, never waits.
    changing: Mutex<()>,
    /// Set to drain: no new connection is taken, and no new plan or apply starts.
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Binds `listen` for `instance`, whose operations use `state`. Requests must carry `token`
    /// as a bearer token or, when `signed`, a signature under it; with `tls`, also a client
    /// certificate its CA issued. Only a loopback address binds unless `network` is set, which
    /// requires `tls` or `signed`.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for a token shorter than 32 characters, a non-loopback address
    /// without `network`, `network` without `tls` or `signed`, or unusable TLS material;
    /// [`Error::Io`] if the address cannot be bound.
    pub fn bind(
        instance: Instance,
        listen: &str,
        state: &str,
        token: &str,
        network: bool,
        tls: Option<&Tls>,
        signed: bool,
    ) -> Result<Self, Error> {
        if token.chars().count() < MIN_TOKEN {
            return Err(Error::Invalid(vec![format!(
                "{TOKEN_VAR} must be at least {MIN_TOKEN} characters"
            )]));
        }
        let io = |e: io::Error| Error::Io {
            path: listen.into(),
            message: e.to_string(),
        };
        let addrs: Vec<SocketAddr> = listen.to_socket_addrs().map_err(io)?.collect();
        let loopback = !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback());
        if !loopback && !network {
            return Err(Error::Invalid(vec![format!(
                "`{listen}` is not a loopback address; bind beyond loopback only with --network"
            )]));
        }
        if network && tls.is_none() && !signed {
            return Err(Error::Invalid(vec![format!(
                "--network requires mutual TLS ({TLS_CERT_VAR}, {TLS_KEY_VAR} and {TLS_CLIENT_CA_VAR}) or --signed"
            )]));
        }
        let tls = tls.map(Tls::config).transpose()?;
        let listener = TcpListener::bind(&addrs[..]).map_err(io)?;
        Ok(Self {
            listener,
            tls,
            deadline: DEADLINE,
            instance,
            state: state.to_string(),
            token: token.to_string(),
            signed,
            seen: Mutex::new(HashMap::new()),
            changing: Mutex::new(()),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Replaces [`DEADLINE`].
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The bound address.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.local_addr().ok()
    }

    /// Answers one request per connection on [`WORKERS`] threads until the process ends;
    /// passphrases and credentials are read through `var`.
    ///
    /// When [`Server::stopper`] is set it drains: it takes no new connection, refuses a new plan or
    /// apply with 503, finishes the requests in hand, and returns (control-plane ADR §7).
    ///
    /// # Panics
    ///
    /// If the listener cannot be made non-blocking.
    pub fn serve(&self, var: impl Fn(&str) -> Option<String> + Sync) {
        self.listener
            .set_nonblocking(true)
            .expect("a bound listener can be made non-blocking");
        std::thread::scope(|scope| {
            for _ in 0..WORKERS {
                scope.spawn(|| {
                    while !self.stopping() {
                        match self.listener.accept() {
                            Ok((stream, _))
                                if !self.stopping() && stream.set_nonblocking(false).is_ok() =>
                            {
                                self.connection(stream, &var);
                            }
                            Ok(_) | Err(_) => std::thread::sleep(POLL),
                        }
                    }
                });
            }
        });
    }

    /// The flag that drains [`Server::serve`].
    #[must_use]
    pub fn stopper(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn connection(&self, stream: TcpStream, var: &impl Fn(&str) -> Option<String>) {
        let _ = stream.set_nodelay(true);
        let mut timed = Timed {
            stream,
            until: Instant::now() + self.deadline,
        };
        let Some(config) = &self.tls else {
            self.exchange(&mut timed, var);
            return;
        };
        let Ok(conn) = ServerConnection::new(Arc::clone(config)) else {
            return;
        };
        let mut tls = StreamOwned::new(conn, timed);
        while tls.conn.is_handshaking() {
            if tls.conn.complete_io(&mut tls.sock).is_err() {
                return;
            }
        }
        self.exchange(&mut tls, var);
        tls.conn.send_close_notify();
        let _ = tls.conn.complete_io(&mut tls.sock);
    }

    /// Reads one request, answers it, then discards a bounded remainder of its body so the
    /// close does not reset the answer.
    fn exchange<S: Read + Write>(&self, s: &mut S, var: &impl Fn(&str) -> Option<String>) {
        let (head, read) = match read_head(s) {
            Ok(h) => h,
            Err(Some((status, body))) => {
                let _ = write_response(s, status, &body.to_string(), None);
                return;
            }
            Err(None) => return,
        };
        let reply = self.answer(&head, read, s, var);
        let text = reply.body.to_string();
        let signature = reply
            .nonce
            .map(|n| sign_response(&self.token, &n, reply.status, text.as_bytes()));
        if write_response(s, reply.status, &text, signature.as_deref()).is_ok() && reply.unread > 0
        {
            let _ = io::copy(&mut s.take(reply.unread.min(2 * MAX_BODY)), &mut io::sink());
        }
    }

    fn authorized(&self, head: &Head) -> bool {
        let want = format!("Bearer {}", self.token);
        head.authorization
            .as_deref()
            .is_some_and(|a| constant_eq(a.as_bytes(), want.as_bytes()))
    }

    /// The reply to `head`; `read` is what arrived after the headers.
    fn answer<S: Read + Write>(
        &self,
        head: &Head,
        read: Vec<u8>,
        s: &mut S,
        var: &impl Fn(&str) -> Option<String>,
    ) -> Reply {
        let unread = head.length.unwrap_or(0).saturating_sub(read.len() as u64);
        let reply = |status, body| Reply {
            status,
            body,
            unread,
            nonce: None,
        };
        match (head.method.as_str(), head.path.as_str()) {
            ("GET", "/healthz") => return reply(200, json!({ "ok": true })),
            (_, "/healthz") => return reply(405, json!({ "error": "method not allowed" })),
            (_, "/v0/operations" | "/v0/run") => {}
            _ => return reply(404, json!({ "error": "not found" })),
        }
        if self.signed {
            return self.signed_answer(head, read, s, var);
        }
        if !self.authorized(head) {
            return reply(401, json!({ "error": "unauthorized" }));
        }
        match (head.method.as_str(), head.path.as_str()) {
            ("GET", "/v0/operations") => reply(200, json!(catalog())),
            ("POST", "/v0/run") => match read_body(head, read, s) {
                Ok(body) => {
                    let (status, body) = self.run_bytes(body, var);
                    Reply {
                        status,
                        body,
                        unread: 0,
                        nonce: None,
                    }
                }
                Err(refused) => refused,
            },
            _ => reply(405, json!({ "error": "method not allowed" })),
        }
    }

    /// A `/v0/` reply in signed mode: the body is read first, since the signature covers it.
    fn signed_answer<S: Read + Write>(
        &self,
        head: &Head,
        read: Vec<u8>,
        s: &mut S,
        var: &impl Fn(&str) -> Option<String>,
    ) -> Reply {
        let body = match read_body(head, read, s) {
            Ok(body) => body,
            Err(refused) => return refused,
        };
        let Some(nonce) = self.verify(head, &body) else {
            return Reply {
                status: 401,
                body: json!({ "error": "unauthorized" }),
                unread: 0,
                nonce: None,
            };
        };
        let (status, body) = match (head.method.as_str(), head.path.as_str()) {
            ("GET", "/v0/operations") => (200, json!(catalog())),
            ("POST", "/v0/run") => self.run_bytes(body, var),
            _ => (405, json!({ "error": "method not allowed" })),
        };
        Reply {
            status,
            body,
            unread: 0,
            nonce: Some(nonce),
        }
    }

    /// The request's nonce when its signature, timestamp and nonce all hold; the nonce is then
    /// spent.
    fn verify(&self, head: &Head, body: &[u8]) -> Option<String> {
        let timestamp: u64 = head.timestamp.as_deref()?.parse().ok()?;
        let nonce = head.nonce.as_deref()?;
        if !(32..=64).contains(&nonce.len()) || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let now = unix_now();
        if now.abs_diff(timestamp) > SKEW {
            return None;
        }
        let want = sign_request(
            &self.token,
            &head.method,
            &head.path,
            timestamp,
            nonce,
            body,
        );
        if !constant_eq(head.signature.as_deref()?.as_bytes(), want.as_bytes()) {
            return None;
        }
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.retain(|_, at| now.saturating_sub(*at) <= 2 * SKEW);
        if seen.insert(nonce.to_ascii_lowercase(), now).is_some() {
            return None;
        }
        Some(nonce.to_string())
    }

    fn run_bytes(&self, body: Vec<u8>, var: &impl Fn(&str) -> Option<String>) -> (u16, Value) {
        match String::from_utf8(body) {
            Ok(text) => self.run(&text, var),
            Err(_) => (400, json!({ "error": "the request body is not UTF-8" })),
        }
    }

    fn run(&self, text: &str, var: &impl Fn(&str) -> Option<String>) -> (u16, Value) {
        let mut request: Request = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(e) => return (400, json!({ "error": format!("not a request: {e}") })),
        };
        if request
            .args
            .get("state")
            .is_some_and(|s| s.as_str() != Some(self.state.as_str()))
        {
            return (
                400,
                json!({ "error": "invalid", "problems": ["`state` is the server's; omit it, or name the server's own"] }),
            );
        }
        if matches!(
            catalog()
                .iter()
                .find(|o| o.name == request.operation)
                .map(|o| o.effect),
            Some(Effect::Plan | Effect::Apply)
        ) && !request.args.contains_key("state")
        {
            request.args.insert("state".into(), json!(self.state));
        }
        let effect = match crate::wire::validate(&request) {
            Ok(op) => Some(op.effect),
            Err(Error::Invalid(problems)) => {
                return (400, json!({ "error": "invalid", "problems": problems }));
            }
            Err(e) => return (500, json!({ "error": e.to_string() })),
        };
        if matches!(effect, Some(Effect::Plan | Effect::Apply)) && self.stopping() {
            return (
                503,
                json!({ "error": "draining: no new plan or apply starts; retry on a new server" }),
            );
        }
        let _guard = if matches!(effect, Some(Effect::Plan | Effect::Apply)) {
            match self.changing.try_lock() {
                Ok(g) => Some(g),
                Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    return (
                        409,
                        json!({ "error": "a plan or apply is already running; retry when it ends" }),
                    );
                }
            }
        } else {
            None
        };
        match run_with(&self.instance, &request, var) {
            Ok(response) => (200, json!(response)),
            Err(Error::Invalid(problems)) => {
                (400, json!({ "error": "invalid", "problems": problems }))
            }
            Err(e) => (500, json!({ "error": e.to_string() })),
        }
    }
}

/// A request's line and the headers `serve` reads.
struct Head {
    method: String,
    path: String,
    authorization: Option<String>,
    length: Option<u64>,
    chunked: bool,
    expect: Option<String>,
    timestamp: Option<String>,
    nonce: Option<String>,
    signature: Option<String>,
}

/// A status and body, how much of the request's body is left unread, and the nonce to sign
/// the response over, for a verified signed request.
struct Reply {
    status: u16,
    body: Value,
    unread: u64,
    nonce: Option<String>,
}

/// The request's whole body, after `expect: 100-continue` if asked; or the reply refusing it.
fn read_body<S: Read + Write>(head: &Head, mut read: Vec<u8>, s: &mut S) -> Result<Vec<u8>, Reply> {
    let length = head.length.unwrap_or(0);
    let unread = length.saturating_sub(read.len() as u64);
    let refuse = |status, body, unread| Reply {
        status,
        body,
        unread,
        nonce: None,
    };
    if head.chunked {
        return Err(refuse(
            411,
            json!({ "error": "the request body needs a content-length" }),
            unread,
        ));
    }
    if head
        .expect
        .as_ref()
        .is_some_and(|e| !e.eq_ignore_ascii_case("100-continue"))
    {
        return Err(refuse(
            417,
            json!({ "error": "only `expect: 100-continue` is supported" }),
            unread,
        ));
    }
    if length > MAX_BODY {
        return Err(refuse(
            413,
            json!({ "error": "the request body is over 1 MiB" }),
            unread,
        ));
    }
    read.truncate(usize::try_from(length).unwrap_or(usize::MAX));
    let unreadable = || {
        refuse(
            400,
            json!({ "error": "the request body could not be read" }),
            0,
        )
    };
    if head.expect.is_some()
        && unread > 0
        && s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .and_then(|()| s.flush())
            .is_err()
    {
        return Err(unreadable());
    }
    match s.take(unread).read_to_end(&mut read) {
        Ok(n) if n as u64 == unread => Ok(read),
        _ => Err(unreadable()),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The answer to a request that was not read: a status and body, or `None` for nothing.
type Refusal = Option<(u16, Value)>;

/// The request's head and whatever followed it.
fn read_head<S: Read>(s: &mut S) -> Result<(Head, Vec<u8>), Refusal> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(None),
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buf) {
            Ok(httparse::Status::Complete(end)) if end > MAX_HEAD => {
                return Err(Some((
                    431,
                    json!({ "error": "the request headers are too large" }),
                )));
            }
            Ok(httparse::Status::Complete(end)) => {
                let header = |name: &str| {
                    request
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case(name))
                        .and_then(|h| std::str::from_utf8(h.value).ok())
                        .map(str::trim)
                };
                let lengths: Vec<&[u8]> = request
                    .headers
                    .iter()
                    .filter(|h| h.name.eq_ignore_ascii_case("content-length"))
                    .map(|h| h.value)
                    .collect();
                let length = match lengths.as_slice() {
                    [] => None,
                    [v] => match std::str::from_utf8(v)
                        .ok()
                        .and_then(|v| v.trim().parse().ok())
                    {
                        Some(n) => Some(n),
                        None => return Err(Some(bad("the content-length is not a number"))),
                    },
                    _ => return Err(Some(bad("more than one content-length"))),
                };
                let head = Head {
                    method: request.method.unwrap_or_default().to_string(),
                    path: request.path.unwrap_or_default().to_string(),
                    authorization: header("authorization").map(str::to_string),
                    length,
                    chunked: header("transfer-encoding").is_some(),
                    expect: header("expect").map(str::to_string),
                    timestamp: header(TIMESTAMP_HEADER).map(str::to_string),
                    nonce: header(NONCE_HEADER).map(str::to_string),
                    signature: header(SIGNATURE_HEADER).map(str::to_string),
                };
                return Ok((head, buf.split_off(end)));
            }
            Ok(httparse::Status::Partial) if buf.len() <= MAX_HEAD => {}
            Ok(httparse::Status::Partial) | Err(httparse::Error::TooManyHeaders) => {
                return Err(Some((
                    431,
                    json!({ "error": "the request headers are too large" }),
                )));
            }
            Err(_) => return Err(Some(bad("not an HTTP/1 request"))),
        }
    }
}

fn bad(why: &str) -> (u16, Value) {
    (400, json!({ "error": why }))
}

fn write_response(
    s: &mut impl Write,
    status: u16,
    text: &str,
    signature: Option<&str>,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        417 => "Expectation Failed",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let signed = signature
        .map(|sig| format!("{SIGNATURE_HEADER}: {sig}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{signed}connection: close\r\n\r\n{text}",
        text.len()
    );
    s.write_all(response.as_bytes())?;
    s.flush()
}

/// A connection whose reads and writes each wait at most [`IO_TIMEOUT`], and whose reads end at
/// `until`.
struct Timed {
    stream: TcpStream,
    until: Instant,
}

impl Timed {
    fn wait(&self) -> io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection's deadline passed",
            ));
        }
        Ok(left.min(IO_TIMEOUT))
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.wait()?))?;
        self.stream.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(IO_TIMEOUT))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
