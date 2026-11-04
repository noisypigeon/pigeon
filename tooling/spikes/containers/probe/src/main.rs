//! Answers the spike's Container checks on port 8080; see ../README.md.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Provider APIs the platform must reach; any HTTP status counts as reachable.
const EGRESS: [(&str, &str); 4] = [
    ("cloudflare", "https://api.cloudflare.com/client/v4/ips"),
    ("scaleway", "https://api.scaleway.com/account/v3/projects"),
    (
        "backblaze",
        "https://api.backblazeb2.com/b2api/v3/b2_authorize_account",
    ),
    ("digitalocean", "https://api.digitalocean.com/v2/account"),
];

fn main() {
    let started = Instant::now();
    let listener = TcpListener::bind("0.0.0.0:8080").expect("bind 0.0.0.0:8080");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || serve(stream, started));
    }
}

fn serve(mut stream: TcpStream, started: Instant) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let (status, body) = match path.split('?').next().unwrap_or("/") {
        "/image" => (200, image()),
        "/uptime" => (200, json!({})),
        "/egress" => (200, egress()),
        _ => (404, json!({ "error": "unknown route" })),
    };
    let mut body = body;
    body["uptime_s"] = json!(started.elapsed().as_secs());
    let text = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn version(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(o) => json!(String::from_utf8_lossy(&o.stdout).trim()),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn image() -> Value {
    json!({
        "tofu": version("tofu", &["version"]),
        "zola": version("zola", &["--version"]),
        "pigeon": version("pigeon", &["--version"]),
    })
}

fn egress() -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut out = serde_json::Map::new();
    for (name, url) in EGRESS {
        let t = Instant::now();
        let result = match agent.get(url).call() {
            Ok(r) => json!({ "status": r.status().as_u16(), "ms": t.elapsed().as_millis() }),
            Err(e) => json!({ "error": e.to_string(), "ms": t.elapsed().as_millis() }),
        };
        out.insert(name.into(), result);
    }
    Value::Object(out)
}
