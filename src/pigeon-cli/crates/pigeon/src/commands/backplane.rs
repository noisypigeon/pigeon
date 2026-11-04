//! `pigeon backplane`; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

use backplane::{Instance, Request, Response};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{BackplaneCommand, RootArgs, ServeArgs};

/// Runs a `pigeon backplane` command and returns its exit code.
#[must_use]
pub fn run(command: BackplaneCommand) -> i32 {
    match command {
        BackplaneCommand::Instance(args) => operation("instance.show", &[], &args),
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
        BackplaneCommand::Run(args) => operation(&args.operation, &args.args, &args.root),
        BackplaneCommand::Serve(args) => serve(&args),
    }
}

fn serve(args: &ServeArgs) -> i32 {
    let token = std::env::var(backplane::serve::TOKEN_VAR).unwrap_or_default();
    let server = match Instance::load(&args.root).and_then(|i| {
        backplane::serve::Server::bind(i, &args.listen, &args.state, &token, args.network)
    }) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("backplane: {e}");
            return INVALID;
        }
    };
    if let Some(addr) = server.local_addr() {
        eprintln!("backplane: serving on {addr}");
    }
    server.serve(|k| std::env::var(k).ok());
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

fn operation(name: &str, raw: &[String], args: &RootArgs) -> i32 {
    let parsed = match parse_args(raw) {
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
    match Instance::load(&args.root).and_then(|i| backplane::run(&i, &request)) {
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
