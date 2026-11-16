//! One fake HTTP server for the modules that talk to an API: a thread per connection, each
//! request parsed once and answered by the module's handler; see
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §1.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

/// A request, as the handler sees it: header names lowercased.
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    /// Header `name`, lowercase.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// An answer: its status line's code and reason, extra headers, and body.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    /// `status` with `value` as JSON.
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: value.to_string().into_bytes(),
        }
    }
}

/// Serves `handler` on a loopback port until the test process ends; answers its base URL.
pub fn serve(handler: impl Fn(Request) -> Reply + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = Arc::clone(&handler);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let target = parts.next().unwrap_or_default().to_string();
                let mut headers = BTreeMap::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim_end().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len = headers
                    .get("content-length")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).expect("body");
                let reply = handler(Request {
                    method,
                    target,
                    headers,
                    body,
                });
                let mut head = format!("HTTP/1.1 {} X\r\n", reply.status);
                for (k, v) in &reply.headers {
                    let _ = write!(head, "{k}: {v}\r\n");
                }
                let _ = write!(
                    head,
                    "content-length: {}\r\nconnection: close\r\n\r\n",
                    reply.body.len()
                );
                let mut out = stream;
                let _ = out.write_all(head.as_bytes());
                let _ = out.write_all(&reply.body);
            });
        }
    });
    format!("http://{addr}")
}
