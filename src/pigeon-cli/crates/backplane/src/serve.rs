//! `pigeon backplane serve`: the wire form over HTTP; see the phase 3 amendment §6 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Mutex;

use serde_json::{Value, json};
use tiny_http::{Header, Method, Response as HttpResponse, Server as Http};

use crate::{Effect, Error, Instance, Request, catalog, run_with};

/// The largest request body read: 1 MiB.
const MAX_BODY: u64 = 1024 * 1024;

/// The environment variable holding the bearer token every `/v0/` request must carry.
pub const TOKEN_VAR: &str = "BACKPLANE_TOKEN";

/// The shortest token accepted.
const MIN_TOKEN: usize = 32;

/// The workers answering requests; more requests wait for one.
pub const WORKERS: usize = 16;

/// A bound server answering one instance's operations against one state.
pub struct Server {
    http: Http,
    instance: Instance,
    state: String,
    token: String,
    /// Held by `plan` or `apply`, which share the state's mirror; a second gets 409, never waits.
    changing: Mutex<()>,
}

impl Server {
    /// Binds `listen` for `instance`, whose operations use `state`; requests must carry
    /// `token` as a bearer token. Only a loopback address binds unless `network` is set, for the
    /// Container's private network that only the Worker reaches.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] for a token shorter than 32 characters, or a non-loopback address
    /// without `network`; [`Error::Io`] if the address cannot be bound.
    pub fn bind(
        instance: Instance,
        listen: &str,
        state: &str,
        token: &str,
        network: bool,
    ) -> Result<Self, Error> {
        if token.chars().count() < MIN_TOKEN {
            return Err(Error::Invalid(vec![format!(
                "{TOKEN_VAR} must be at least {MIN_TOKEN} characters"
            )]));
        }
        let loopback = listen
            .to_socket_addrs()
            .is_ok_and(|mut a| a.all(|a| a.ip().is_loopback()));
        if !loopback && !network {
            return Err(Error::Invalid(vec![format!(
                "`{listen}` is not a loopback address; plain HTTP carries the token, so bind beyond loopback only on the Container's private network, with --network"
            )]));
        }
        let http = Http::http(listen).map_err(|e| Error::Io {
            path: listen.into(),
            message: e.to_string(),
        })?;
        Ok(Self {
            http,
            instance,
            state: state.to_string(),
            token: token.to_string(),
            changing: Mutex::new(()),
        })
    }

    /// The bound address.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.http.server_addr().to_ip()
    }

    /// Answers requests on [`WORKERS`] threads until the process ends; passphrases and
    /// credentials are read through `var`.
    pub fn serve(&self, var: impl Fn(&str) -> Option<String> + Sync) {
        std::thread::scope(|scope| {
            for _ in 0..WORKERS {
                scope.spawn(|| {
                    while let Ok(request) = self.http.recv() {
                        self.answer(request, &var);
                    }
                });
            }
        });
    }

    fn authorized(&self, request: &tiny_http::Request) -> bool {
        let want = format!("Bearer {}", self.token);
        request
            .headers()
            .iter()
            .find(|h| h.field.equiv("authorization"))
            .is_some_and(|h| constant_eq(h.value.as_str().as_bytes(), want.as_bytes()))
    }

    fn answer(&self, mut request: tiny_http::Request, var: &impl Fn(&str) -> Option<String>) {
        let url = request.url().to_string();
        let (status, body) = match (request.method(), url.as_str()) {
            (Method::Get, "/healthz") => (200, json!({ "ok": true })),
            (_, "/v0/operations" | "/v0/run") if !self.authorized(&request) => {
                (401, json!({ "error": "unauthorized" }))
            }
            (Method::Get, "/v0/operations") => (200, json!(catalog())),
            (Method::Post, "/v0/run") => {
                let mut bytes = Vec::new();
                match request
                    .as_reader()
                    .take(MAX_BODY + 1)
                    .read_to_end(&mut bytes)
                {
                    Ok(n) if n as u64 > MAX_BODY => {
                        (413, json!({ "error": "the request body is over 1 MiB" }))
                    }
                    Ok(_) => match String::from_utf8(bytes) {
                        Ok(text) => self.run(&text, var),
                        Err(_) => (400, json!({ "error": "the request body is not UTF-8" })),
                    },
                    Err(_) => (
                        400,
                        json!({ "error": "the request body could not be read" }),
                    ),
                }
            }
            (_, "/healthz" | "/v0/operations" | "/v0/run") => {
                (405, json!({ "error": "method not allowed" }))
            }
            _ => (404, json!({ "error": "not found" })),
        };
        respond(request, status, &body);
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
        let effect = match crate::catalog::validate(&request) {
            Ok(op) => Some(op.effect),
            Err(Error::Invalid(problems)) => {
                return (400, json!({ "error": "invalid", "problems": problems }));
            }
            Err(e) => return (500, json!({ "error": e.to_string() })),
        };
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

fn respond(request: tiny_http::Request, status: u16, body: &Value) {
    let header = Header::from_bytes(&b"content-type"[..], &b"application/json"[..])
        .expect("a static header is valid");
    let response = HttpResponse::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header);
    let _ = request.respond(response);
}

/// Compares in time independent of where the inputs first differ.
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
