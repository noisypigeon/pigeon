//! Answers the spike's Container checks on port 8080, and a TLS check on 8443; see ../README.md.

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
    if let Some(config) = tls_config() {
        std::thread::spawn(move || serve_tls(&config));
    } else if std::env::var_os("SPIKE_TLS_CERT").is_some() {
        eprintln!("spike-probe: SPIKE_TLS_* set but unusable; no TLS listener");
    }
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

/// The TLS listener's config from `SPIKE_TLS_CERT`, `SPIKE_TLS_KEY` and `SPIKE_TLS_CLIENT_CA`
/// (PEM); client certificates are optional so the check can report whether one arrived.
fn tls_config() -> Option<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (cert, key, ca) = (
        var("SPIKE_TLS_CERT")?,
        var("SPIKE_TLS_KEY")?,
        var("SPIKE_TLS_CLIENT_CA")?,
    );
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert.as_bytes())
        .collect::<Result<_, _>>()
        .ok()?;
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).ok()?;
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(ca.as_bytes()) {
        roots.add(c.ok()?).ok()?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::clone(&provider),
    )
    .allow_unauthenticated()
    .build()
    .ok()?;
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .ok()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .ok()?;
    Some(Arc::new(config))
}

/// Answers every TLS request on 8443 with whether the handshake carried a client certificate.
fn serve_tls(config: &Arc<rustls::ServerConfig>) {
    let Ok(listener) = TcpListener::bind("0.0.0.0:8443") else {
        return;
    };
    for stream in listener.incoming().flatten() {
        let config = Arc::clone(config);
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let Ok(conn) = rustls::ServerConnection::new(config) else {
                return;
            };
            let mut tls = rustls::StreamOwned::new(conn, stream);
            let mut line = String::new();
            if BufReader::new(&mut tls).read_line(&mut line).is_err() {
                return;
            }
            let client = tls.conn.peer_certificates().is_some_and(|c| !c.is_empty());
            let text = json!({
                "tls": true,
                "version": format!("{:?}", tls.conn.protocol_version()),
                "client_certificate": client,
            })
            .to_string();
            let _ = write!(
                tls,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            tls.conn.send_close_notify();
            let _ = tls.flush();
        });
    }
}
