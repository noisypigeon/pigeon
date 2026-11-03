//! `pigeon backplane`; see `.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md`.

use backplane::{Instance, Request, Response};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{BackplaneCommand, RootArgs};

/// Runs a `pigeon backplane` command and returns its exit code.
#[must_use]
pub fn run(command: BackplaneCommand) -> i32 {
    match command {
        BackplaneCommand::Instance(args) => operation("instance.show", &args),
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
        BackplaneCommand::Run(args) => operation(&args.operation, &args.root),
    }
}

fn operation(name: &str, args: &RootArgs) -> i32 {
    let request = Request {
        operation: name.to_string(),
        ..Request::default()
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
