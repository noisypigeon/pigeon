//! `pigeon backplane`; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

use backplane::{Instance, Request, Response};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{BackplaneCommand, JobsArgs, RootArgs, ServeArgs};

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
        BackplaneCommand::Serve(args) => serve(&args),
        BackplaneCommand::Jobs(args) => jobs(&args),
    }
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
