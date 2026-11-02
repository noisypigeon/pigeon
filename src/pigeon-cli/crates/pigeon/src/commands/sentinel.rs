//! `pigeon sentinel`; see `.claude/decisions/2026-11-02-development-cargo-workspace-mise-toolchain.md`.

use std::env;

use sentinel::PullRequest;
use sentinel::release::{self, Config};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{CheckArgs, ReleaseArgs, SentinelCommand, TagsArgs};

/// Runs a `pigeon sentinel` command and returns its exit code.
#[must_use]
pub fn run(command: SentinelCommand) -> i32 {
    match command {
        SentinelCommand::Check(args) => check(&args),
        SentinelCommand::Release(args) => release(&args),
        SentinelCommand::Tags(args) => tags(&args),
    }
}

fn check(args: &CheckArgs) -> i32 {
    let dir = args
        .policies
        .clone()
        .unwrap_or_else(|| args.root.join(".config/policies"));
    let policies = match sentinel::load_dir(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("sentinel: {e}");
            return INVALID;
        }
    };
    let pull_request = args.pull_request.then(|| PullRequest {
        title: env::var("PR_TITLE").ok(),
        body: env::var("PR_BODY").ok(),
        number: env::var("PR_NUMBER").ok(),
        files: env::var("PR_FILES").ok(),
    });
    match sentinel::check(&policies, &args.root, pull_request.as_ref()) {
        Ok(report) => {
            for v in &report.violations {
                println!("{v}");
            }
            println!("{}", report.summary());
            if report.passed() { SUCCESS } else { VIOLATION }
        }
        Err(e) => {
            eprintln!("sentinel: {e}");
            INVALID
        }
    }
}

fn print_json<T: serde::Serialize>(result: Result<T, sentinel::Error>) -> i32 {
    match result.map(|v| serde_json::to_string(&v)) {
        Ok(Ok(json)) => {
            println!("{json}");
            SUCCESS
        }
        Ok(Err(e)) => {
            eprintln!("sentinel: {e}");
            INVALID
        }
        Err(e) => {
            eprintln!("sentinel: {e}");
            INVALID
        }
    }
}

fn tags(args: &TagsArgs) -> i32 {
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(".config/releases.yaml"));
    print_json(Config::load(&path).and_then(|config| release::tags(&args.root, &config)))
}

fn release(args: &ReleaseArgs) -> i32 {
    let path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(".config/releases.yaml"));
    print_json(Config::load(&path).and_then(|config| {
        release::run(
            &args.root,
            &config,
            args.from.as_deref(),
            &args.to,
            args.notes_dir.as_deref(),
            args.dry_run,
        )
    }))
}
