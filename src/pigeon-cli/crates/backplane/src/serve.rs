//! `pigeon backplane serve`: the wire form over HTTP; see the phase 3 amendment §6 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::io::Read;
use std::net::SocketAddr;

use serde_json::{Value, json};
use tiny_http::{Header, Method, Response as HttpResponse, Server as Http};

use crate::{Error, Instance, Request, catalog, run_with};

/// The largest request body read: 1 MiB.
const MAX_BODY: u64 = 1024 * 1024;

/// A bound server answering one instance's operations.
pub struct Server {
    http: Http,
    instance: Instance,
}

impl Server {
    /// Binds `listen`, such as `127.0.0.1:8787` or `0.0.0.0:8080` inside the Container.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the address cannot be bound.
    pub fn bind(instance: Instance, listen: &str) -> Result<Self, Error> {
        let http = Http::http(listen).map_err(|e| Error::Io {
            path: listen.into(),
            message: e.to_string(),
        })?;
        Ok(Self { http, instance })
    }

    /// The bound address.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.http.server_addr().to_ip()
    }

    /// Answers requests until the process ends, reading passphrases and credentials through
    /// `var`.
    pub fn serve(&self, var: impl Fn(&str) -> Option<String>) {
        for request in self.http.incoming_requests() {
            self.answer(request, &var);
        }
    }

    fn answer(&self, mut request: tiny_http::Request, var: &impl Fn(&str) -> Option<String>) {
        let (status, body) = match (request.method(), request.url()) {
            (Method::Get, "/healthz") => (200, json!({ "ok": true })),
            (Method::Get, "/v0/operations") => (200, json!(catalog())),
            (Method::Post, "/v0/run") => {
                let mut text = String::new();
                match request
                    .as_reader()
                    .take(MAX_BODY + 1)
                    .read_to_string(&mut text)
                {
                    Ok(n) if n as u64 > MAX_BODY => {
                        (413, json!({ "error": "the request body is over 1 MiB" }))
                    }
                    Ok(_) => self.run(&text, var),
                    Err(_) => (400, json!({ "error": "the request body is not UTF-8" })),
                }
            }
            (_, "/healthz" | "/v0/operations" | "/v0/run") => {
                (405, json!({ "error": "method not allowed" }))
            }
            _ => (404, json!({ "error": "not found" })),
        };
        let header = Header::from_bytes(&b"content-type"[..], &b"application/json"[..])
            .expect("a static header is valid");
        let response = HttpResponse::from_string(body.to_string())
            .with_status_code(status)
            .with_header(header);
        let _ = request.respond(response);
    }

    fn run(&self, text: &str, var: &impl Fn(&str) -> Option<String>) -> (u16, Value) {
        let request: Request = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(e) => return (400, json!({ "error": format!("not a request: {e}") })),
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
