//! `pigeon session`: the shell around `backplane::core::session`, through the agent's proxy,
//! which authenticates the host; see
//! `.claude/decisions/2026-11-13-development-rust-session-event-follower.md` and
//! `.claude/decisions/2026-11-15-development-no-python-rust-tooling.md`.

use std::hash::BuildHasher;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backplane::core::session::launch;
use backplane::core::session::{self as core, Effect, Follow, HOLD, Outcome, Reply, START, Status};
use serde_json::{Value, json};

use crate::cli::SessionCommand;

/// How long `brief` waits for the control plane, in seconds.
const BRIEF: u64 = 10;
/// How long a `session.status` call waits from the follower, in seconds.
const STATUS: u64 = 30;
/// How long `install` waits for `deploy.status`, in seconds.
const DEPLOY: u64 = 10;
/// How long `install` waits for the build, in seconds.
const FETCH: u64 = 60;

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
        SessionCommand::Executor { brief, tags } => executor(brief, tags),
        SessionCommand::Install => install(),
        SessionCommand::Bench { command } => crate::commands::bench::run(command),
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
    let mut result = call(agent, "session.status", &json!({ "session": me }))?;
    result.as_object_mut()?.insert("at".into(), now().into());
    let path = cache()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&tmp, result.to_string()).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    serde_json::from_value(result).ok()
}

/// Runs `operation` on the control plane: its `result`, or `None`.
fn call(agent: &ureq::Agent, operation: &str, args: &Value) -> Option<Value> {
    let body = json!({ "operation": operation, "args": args });
    let url = format!("https://{}/v0/run", host());
    let text = agent
        .post(&url)
        .header("content-type", "application/json")
        .send(body.to_string())
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let result = serde_json::from_str::<Value>(&text).ok()?["result"].take();
    (!result.is_null()).then_some(result)
}

/// The repository root: the nearest ancestor of the working directory with
/// `tooling/session/models.json`.
fn root() -> Option<PathBuf> {
    let here = std::env::current_dir().ok()?;
    here.ancestors()
        .find(|d| d.join("tooling/session/models.json").is_file())
        .map(PathBuf::from)
}

fn executor(want: &str, tags: &[String]) -> i32 {
    let found = root().and_then(|r| {
        let config = std::fs::read_to_string(r.join("config.yaml")).ok()?;
        let models = std::fs::read_to_string(r.join("tooling/session/models.json")).ok()?;
        Some((
            launch::repository(&config),
            serde_json::from_str::<Value>(&models).ok()?,
        ))
    });
    let Some((source, models)) = found else {
        eprintln!("no pigeon checkout here: run it from the repository");
        return 1;
    };
    let Some(source) = source else {
        eprintln!("{}", launch::NO_REPOSITORY);
        return 1;
    };
    let brief =
        call(&agent(STATUS), "brief.show", &json!({ "id": want.trim() })).unwrap_or_default();
    match launch::executor(want.trim(), &brief, &models, &source, tags) {
        Ok(args) => match serde_json::to_string_pretty(&args) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// Fetches main's newest build to `~/.cache/pigeon/bin/pigeon`, SHA-256 checked; nothing when
/// it is there.
fn install() -> i32 {
    let status = call(&agent(DEPLOY), "deploy.status", &json!({}));
    let Some((sha, want)) = status.as_ref().and_then(launch::latest_build) else {
        eprintln!("deploy.status names no build");
        return 1;
    };
    let Some(bin) = home().map(|h| h.join(".cache/pigeon/bin/pigeon")) else {
        return 1;
    };
    let mark = bin.with_extension("sha");
    let have = std::fs::read_to_string(&mark).unwrap_or_default();
    if have.trim() == sha && executable(&bin) {
        return 0;
    }
    match fetch(&sha, &want, &bin) {
        Ok(()) => {
            let _ = std::fs::write(&mark, &sha);
            0
        }
        Err(why) => {
            eprintln!("install: {why}");
            1
        }
    }
}

fn fetch(sha: &str, want: &str, bin: &Path) -> Result<(), String> {
    let url = format!("https://{}/builds/{sha}/pigeon", host());
    let mut r = agent(FETCH).get(&url).call().map_err(|e| e.to_string())?;
    if r.status().as_u16() != 200 {
        return Err(format!("{url}: {}", r.status()));
    }
    let bytes = r
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    if !launch::verified(&bytes, want) {
        return Err(format!("{url}: not the SHA-256 deploy.status names"));
    }
    let dir = bin.parent().ok_or("no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = bin.with_extension("tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    let moved = mark_executable(&tmp).and_then(|()| std::fs::rename(&tmp, bin));
    if let Err(e) = moved {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(unix)]
fn mark_executable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn mark_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

pub(crate) fn agent(seconds: u64) -> ureq::Agent {
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

pub(crate) fn host() -> String {
    std::env::var("PIGEON_HOST").unwrap_or_else(|_| "platform.pigeon.dev".into())
}

pub(crate) fn home() -> Option<PathBuf> {
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
