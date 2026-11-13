//! `pigeon session`: the shell around `backplane::core::session`, through the agent's proxy,
//! which authenticates the host; see
//! `.claude/decisions/2026-11-13-development-rust-session-event-follower.md`.

use std::hash::BuildHasher;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backplane::core::session::{self as core, Effect, Follow, HOLD, Outcome, Reply, START, Status};
use serde_json::{Value, json};

use crate::cli::SessionCommand;

/// How long `brief` waits for the control plane, in seconds.
const BRIEF: u64 = 10;
/// How long a `session.status` call waits from the follower, in seconds.
const STATUS: u64 = 30;

/// Runs a `pigeon session` command and returns its exit code.
#[must_use]
pub fn run(command: &SessionCommand) -> i32 {
    match command {
        SessionCommand::Follow => follow(),
        SessionCommand::Brief => {
            brief();
            0
        }
        SessionCommand::Statusline => {
            drain();
            println!(
                "{}",
                cached().map_or_else(
                    || "pigeon: no session status yet".into(),
                    |s| core::statusline(&s, now())
                )
            );
            0
        }
        SessionCommand::Stop => {
            drain();
            if let Some(said) = cached().as_ref().and_then(core::stop) {
                println!("{said}");
            }
            0
        }
    }
}

fn follow() -> i32 {
    let Some(me) = label() else {
        println!("{}", START.lines().next().unwrap_or_default());
        return 1;
    };
    let quit = Arc::new(AtomicBool::new(true));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        let _ = signal_hook::flag::register_conditional_shutdown(signal, 0, Arc::clone(&quit));
    }
    let events = agent(HOLD + 15);
    let status = agent(STATUS);
    let mut s = Follow::default();
    loop {
        let outcome = poll(&events, &me, &s);
        let (next, effects) = core::step(&s, outcome, now(), random());
        s = next;
        for e in effects {
            match e {
                Effect::Print(line) => println!("{line}"),
                Effect::Status => {
                    refresh(&status, &me);
                }
                Effect::Sleep(n) => std::thread::sleep(Duration::from_secs(n)),
                Effect::Exit(code) => return code,
            }
        }
    }
}

fn poll(agent: &ureq::Agent, me: &str, s: &Follow) -> Outcome {
    let url = format!("https://{}{}", host(), core::path(me, s.cursor, s.hold));
    match agent.get(&url).call() {
        Err(e) => Outcome::Unreachable(e.to_string()),
        Ok(mut r) => {
            let status = r.status().as_u16();
            let content_type = r
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let cloudflare = r.headers().contains_key("cf-ray");
            let body = r.body_mut().read_to_string().unwrap_or_default();
            core::classify(&Reply {
                status,
                content_type: &content_type,
                cloudflare,
                body: &body,
            })
        }
    }
}

fn brief() {
    let Some(me) = label() else {
        println!("{START}");
        return;
    };
    let fresh = refresh(&agent(BRIEF), &me);
    let s = fresh.or_else(cached);
    println!("{}", core::brief(&me, s.as_ref()));
}

/// The session's status, fresh, saved for the status line; `None` when unreachable.
fn refresh(agent: &ureq::Agent, me: &str) -> Option<Status> {
    let body = json!({ "operation": "session.status", "args": { "session": me } });
    let url = format!("https://{}/v0/run", host());
    let text = agent
        .post(&url)
        .header("content-type", "application/json")
        .send(body.to_string())
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let mut result = serde_json::from_str::<Value>(&text).ok()?["result"].take();
    result.as_object_mut()?.insert("at".into(), now().into());
    let path = cache()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&tmp, result.to_string()).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    serde_json::from_value(result).ok()
}

fn agent(seconds: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(seconds)))
        .tls_config(tls())
        .build()
        .into()
}

/// `SSL_CERT_FILE`'s roots when it is set, as the agent's proxy needs; else webpki's.
fn tls() -> ureq::tls::TlsConfig {
    let roots: Vec<_> = std::env::var_os("SSL_CERT_FILE")
        .and_then(|p| std::fs::read(p).ok())
        .map(|pem| {
            ureq::tls::parse_pem(&pem)
                .filter_map(|item| match item {
                    Ok(ureq::tls::PemItem::Certificate(c)) => Some(c),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let builder = ureq::tls::TlsConfig::builder();
    if roots.is_empty() {
        builder.build()
    } else {
        builder
            .root_certs(ureq::tls::RootCerts::Specific(Arc::new(roots)))
            .build()
    }
}

fn host() -> String {
    std::env::var("PIGEON_HOST").unwrap_or_else(|_| "platform.pigeon.dev".into())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn label() -> Option<String> {
    let path = home()?.join(".config/pigeon/session.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    v["label"].as_str().map(str::to_string)
}

fn cache() -> Option<PathBuf> {
    home().map(|h| h.join(".cache/pigeon/status.json"))
}

fn cached() -> Option<Status> {
    serde_json::from_str(&std::fs::read_to_string(cache()?).ok()?).ok()
}

/// Reads a hook's JSON from stdin, which it doesn't need, so the harness never blocks on it.
fn drain() {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let _ = stdin.lock().read_to_end(&mut Vec::new());
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn random() -> u32 {
    let [a, b, c, d, ..] = std::collections::hash_map::RandomState::new()
        .hash_one(())
        .to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}
