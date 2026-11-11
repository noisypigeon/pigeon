//! `pigeon backplane`; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

use backplane::{Instance, Request, Response};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{BackplaneCommand, DecisionsArgs, JobsArgs, RootArgs, ServeArgs};

/// Runs a `pigeon backplane` command and returns its exit code.
#[must_use]
pub fn run(command: BackplaneCommand) -> i32 {
    match command {
        BackplaneCommand::Instance(args) => operation("instance.show", &[], &[], &args),
        BackplaneCommand::Engines(args) => {
            let engines = backplane::engines();
            if args.json {
                print_json(&engines);
            } else {
                for e in &engines {
                    println!("{} {} {}", e.engine.name(), e.version, e.role);
                }
            }
            SUCCESS
        }
        BackplaneCommand::Operations(args) => {
            let ops = backplane::catalog();
            if args.json {
                print_json(&ops);
            } else {
                for o in ops {
                    println!(
                        "{} ({}, {}): {}",
                        o.name,
                        o.engine.name(),
                        o.effect.name(),
                        o.summary
                    );
                }
            }
            SUCCESS
        }
        BackplaneCommand::Run(args) => {
            operation(&args.operation, &args.args, &args.text, &args.root)
        }
        BackplaneCommand::Decisions(args) => decisions(&args),
        BackplaneCommand::Serve(args) => serve(&args),
        BackplaneCommand::Jobs(args) => jobs(&args),
        BackplaneCommand::TotpSecret => totp_secret(),
        BackplaneCommand::VapidKey => vapid_key(),
        #[cfg(feature = "signing")]
        BackplaneCommand::Signing(c) => super::signing::run(c),
    }
}

/// 20 random bytes as base32 on stdout, and their `otpauth://` URI on stderr.
fn totp_secret() -> i32 {
    use std::io::Read;
    let mut bytes = [0u8; 20];
    if let Err(e) = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)) {
        eprintln!("backplane: no randomness: {e}");
        return INVALID;
    }
    let secret = backplane::totp::base32_encode(&bytes);
    eprintln!(
        "Add this to your authenticator app (it is shown once; nothing stores it but the app and the Worker):\n\
         otpauth://totp/pigeon:owner?secret={secret}&issuer=pigeon&algorithm=SHA1&digits=6&period=30"
    );
    println!("{secret}");
    SUCCESS
}

/// A random P-256 private key, base64url, on stdout, and its public key on stderr.
fn vapid_key() -> i32 {
    use std::io::Read;
    for _ in 0..8 {
        let mut bytes = [0u8; 32];
        if let Err(e) =
            std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes))
        {
            eprintln!("backplane: no randomness: {e}");
            return INVALID;
        }
        let secret = backplane::push::b64url(&bytes);
        if let Ok(v) = backplane::push::Vapid::from_secret(&secret) {
            eprintln!("Web Push public key: {}", v.public_key());
            println!("{secret}");
            return SUCCESS;
        }
    }
    eprintln!("backplane: no valid key after 8 tries");
    INVALID
}

fn serve(args: &ServeArgs) -> i32 {
    let token = std::env::var(backplane::serve::TOKEN_VAR).unwrap_or_default();
    let bound = backplane::serve::Tls::from_vars(|k| std::env::var(k).ok()).and_then(|tls| {
        let instance = load(&args.root, args.instance.as_deref())?;
        backplane::serve::Server::bind(
            instance,
            &args.listen,
            &args.state,
            &token,
            args.network,
            tls.as_ref(),
            args.signed,
        )
    });
    let server = match bound {
        Ok(s) => s,
        Err(e) => {
            eprintln!("backplane: {e}");
            return INVALID;
        }
    };
    if let Some(addr) = server.local_addr() {
        eprintln!("backplane: serving on {addr}");
    }
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(e) = signal_hook::flag::register(signal, server.stopper()) {
            eprintln!("backplane: cannot handle signal {signal}: {e}");
            return INVALID;
        }
    }
    server.serve(|k| std::env::var(k).ok());
    eprintln!("backplane: stopped after finishing its requests");
    SUCCESS
}

fn jobs(args: &JobsArgs) -> i32 {
    let token = std::env::var(backplane::serve::TOKEN_VAR).unwrap_or_default();
    let board = std::sync::Arc::new(backplane::jobs::Board::new(args.work.clone()));
    let server = match backplane::serve::Server::bind_jobs(&args.listen, &token, board) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("backplane: {e}");
            return INVALID;
        }
    };
    if let Some(addr) = server.local_addr() {
        eprintln!("backplane: taking jobs on {addr}");
    }
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(e) = signal_hook::flag::register(signal, server.stopper()) {
            eprintln!("backplane: cannot handle signal {signal}: {e}");
            return INVALID;
        }
    }
    server.serve(|_| None);
    SUCCESS
}

/// Parses `name=value` arguments; a value that is valid JSON is JSON, else a string.
fn parse_args(raw: &[String]) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    raw.iter()
        .map(|a| {
            let (k, v) = a
                .split_once('=')
                .ok_or_else(|| format!("`--arg {a}` must be name=value"))?;
            let value = serde_json::from_str(v)
                .unwrap_or_else(|_| serde_json::Value::String(v.to_string()));
            Ok((k.to_string(), value))
        })
        .collect()
}

fn operation(name: &str, raw: &[String], text: &[String], args: &RootArgs) -> i32 {
    let parsed = match parse_args(raw).and_then(|mut p| {
        for t in text {
            let (k, v) = t
                .split_once('=')
                .ok_or_else(|| format!("`--text {t}` must be name=value"))?;
            p.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        Ok(p)
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("backplane: {e}");
            return INVALID;
        }
    };
    let request = Request {
        operation: name.to_string(),
        args: parsed,
    };
    match load(&args.root, args.instance.as_deref()).and_then(|i| backplane::run(&i, &request)) {
        Ok(response) => {
            if args.json {
                print_json(&response);
            } else {
                print_text(&response);
            }
            if response.ok { SUCCESS } else { VIOLATION }
        }
        Err(e) => {
            eprintln!("backplane: {e}");
            INVALID
        }
    }
}

fn print_text(response: &Response) {
    for v in &response.violations {
        println!("{v}");
    }
    let state = if response.ok { "ok" } else { "violations" };
    println!(
        "backplane: {} {state}: {}",
        response.operation, response.result
    );
}

/// `pigeon backplane decisions`: the ADRs governing each path.
fn decisions(args: &DecisionsArgs) -> i32 {
    let dir = args.root.join(backplane::decisions::DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("backplane: {}: {e}", dir.display());
            return INVALID;
        }
    };
    let mut all = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_md = std::path::Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if !is_md || name == "README.md" {
            continue;
        }
        match std::fs::read_to_string(entry.path()) {
            Ok(text) => all.push(backplane::decisions::parse(&name, &text)),
            Err(e) => {
                eprintln!("backplane: {name}: {e}");
                return INVALID;
            }
        }
    }
    all.sort_by(|a, b| a.file.cmp(&b.file));
    let found: Vec<(&str, Vec<&backplane::decisions::Decision>)> = args
        .paths
        .iter()
        .map(|p| (p.as_str(), backplane::decisions::governing(&all, p)))
        .collect();
    if args.json {
        let v: serde_json::Map<String, serde_json::Value> = found
            .iter()
            .map(|(p, ds)| ((*p).to_string(), serde_json::json!(ds)))
            .collect();
        print_json(&v);
    } else {
        for (p, ds) in &found {
            println!("{p}:");
            for d in ds {
                println!("  {} ({})", d.file, d.title);
            }
        }
    }
    SUCCESS
}

fn print_json<T: serde::Serialize + ?Sized>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("backplane: {e}"),
    }
}

/// The instance at `instance`, on the platform at `root`; or the one at `root`.
fn load(
    root: &std::path::Path,
    instance: Option<&std::path::Path>,
) -> Result<Instance, backplane::Error> {
    match instance {
        Some(dir) => Instance::load_on(dir, Some(root)),
        None => Instance::load(root),
    }
}
