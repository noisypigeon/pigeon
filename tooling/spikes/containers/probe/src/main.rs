//! Answers the spike's Container checks on port 8080; see ../README.md.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
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

/// Seconds between a timed job's heartbeats.
const BEAT_SECS: u64 = 30;

/// A timed job: busy work with a heartbeat every [`BEAT_SECS`], standing in for an apply.
#[derive(Default)]
struct Job {
    minutes: u64,
    started: Option<Instant>,
    beats: u64,
    longest_gap_s: u64,
    done: bool,
}

fn main() {
    let started = Instant::now();
    let job = Arc::new(Mutex::new(Job::default()));
    let listener = TcpListener::bind("0.0.0.0:8080").expect("bind 0.0.0.0:8080");
    for stream in listener.incoming().flatten() {
        let job = Arc::clone(&job);
        std::thread::spawn(move || serve(stream, started, &job));
    }
}

fn serve(mut stream: TcpStream, started: Instant, job: &Arc<Mutex<Job>>) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let (status, body) = match route {
        "/image" => (200, image()),
        "/uptime" => (200, json!({})),
        "/egress" => (200, egress()),
        "/run" => (200, start(job, minutes(query))),
        "/status" => (200, status(job)),
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

fn minutes(query: &str) -> u64 {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("minutes="))
        .and_then(|m| m.parse().ok())
        .filter(|m| (1..=120).contains(m))
        .unwrap_or(20)
}

/// Starts the timed job unless one is running or done.
fn start(job: &Arc<Mutex<Job>>, minutes: u64) -> Value {
    {
        let mut j = job.lock().expect("job lock");
        if j.started.is_none() {
            *j = Job {
                minutes,
                started: Some(Instant::now()),
                ..Job::default()
            };
            let job = Arc::clone(job);
            std::thread::spawn(move || run(&job, minutes));
        }
    }
    status(job)
}

/// Works for `minutes`, recording each heartbeat and the longest gap between them.
fn run(job: &Arc<Mutex<Job>>, minutes: u64) {
    let end = Instant::now() + Duration::from_secs(minutes * 60);
    let mut last = Instant::now();
    while Instant::now() < end {
        let beat = Instant::now() + Duration::from_secs(BEAT_SECS);
        let mut x: u64 = 0;
        while Instant::now() < beat {
            x = std::hint::black_box(x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1));
        }
        let _ = version("tofu", &["version"]);
        let mut j = job.lock().expect("job lock");
        j.beats += 1;
        j.longest_gap_s = j.longest_gap_s.max(last.elapsed().as_secs());
        last = Instant::now();
    }
    job.lock().expect("job lock").done = true;
}

fn status(job: &Arc<Mutex<Job>>) -> Value {
    let j = job.lock().expect("job lock");
    json!({
        "job": {
            "minutes": j.minutes,
            "running": j.started.is_some() && !j.done,
            "done": j.done,
            "elapsed_s": j.started.map_or(0, |s| s.elapsed().as_secs()),
            "beats": j.beats,
            "expected_beats": j.minutes * 60 / BEAT_SECS,
            "longest_gap_s": j.longest_gap_s,
        }
    })
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
