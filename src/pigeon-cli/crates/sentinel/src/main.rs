//! `pigeon-sentinel`: judges a tree against policies and reports; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §1-§2.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use sentinel::PullRequest;

/// Every policy held.
const SUCCESS: u8 = 0;
/// A policy was violated.
const VIOLATION: u8 = 1;
/// A policy or an argument was invalid.
const INVALID: u8 = 2;

const USAGE: &str = "usage: pigeon-sentinel check [--root DIR] [--policies DIR] [--pull-request] [--json]\n       pigeon-sentinel describe [--root DIR] [--policies DIR]\n       pigeon-sentinel --version";

/// `check`'s arguments.
#[derive(Debug, PartialEq, Eq)]
struct Check {
    root: PathBuf,
    policies: Option<PathBuf>,
    pull_request: bool,
    json: bool,
}

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Check(Check),
    /// Each policy and its target, as JSON; `check`'s flags but the pull request and `--json`.
    Describe(Check),
    Version,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let describing = match args.first().map(String::as_str) {
        Some("--version" | "-V") => return Ok(Command::Version),
        Some("check") => false,
        Some("describe") => true,
        _ => return Err(USAGE.into()),
    };
    let mut check = Check {
        root: PathBuf::from("."),
        policies: None,
        pull_request: false,
        json: false,
    };
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        let mut value = |flag: &str| {
            rest.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{flag} needs a directory\n{USAGE}"))
        };
        match a.as_str() {
            "--root" => check.root = value("--root")?,
            "--policies" => check.policies = Some(value("--policies")?),
            "--pull-request" if !describing => check.pull_request = true,
            "--json" if !describing => check.json = true,
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(if describing {
        Command::Describe(check)
    } else {
        Command::Check(check)
    })
}

fn describe(check: &Check) -> u8 {
    let dir = check
        .policies
        .clone()
        .unwrap_or_else(|| check.root.join(".config/policies"));
    match sentinel::load_dir(&dir) {
        Ok(policies) => {
            println!("{}", sentinel::describe(&policies));
            SUCCESS
        }
        Err(e) => {
            eprintln!("sentinel: {e}");
            INVALID
        }
    }
}

fn run(check: &Check) -> u8 {
    let dir = check
        .policies
        .clone()
        .unwrap_or_else(|| check.root.join(".config/policies"));
    let policies = match sentinel::load_dir(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("sentinel: {e}");
            return INVALID;
        }
    };
    let pull_request = check.pull_request.then(|| PullRequest {
        title: env::var("PR_TITLE").ok(),
        body: env::var("PR_BODY").ok(),
        number: env::var("PR_NUMBER").ok(),
        files: env::var("PR_FILES").ok(),
        branch: env::var("PR_BRANCH").ok(),
        scope: env::var("PR_SCOPE").ok(),
    });
    match sentinel::check(&policies, &check.root, pull_request.as_ref()) {
        Ok(report) => {
            if check.json {
                let violations: Vec<String> =
                    report.violations.iter().map(ToString::to_string).collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "ran": report.ran,
                        "skipped": report.skipped,
                        "violations": violations,
                    })
                );
            } else {
                for v in &report.violations {
                    println!("{v}");
                }
                println!("{}", report.summary());
            }
            if report.passed() { SUCCESS } else { VIOLATION }
        }
        Err(e) => {
            eprintln!("sentinel: {e}");
            INVALID
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    ExitCode::from(match parse(&args) {
        Ok(Command::Version) => {
            println!("pigeon-sentinel {}", sentinel::VERSION);
            SUCCESS
        }
        Ok(Command::Check(check)) => run(&check),
        Ok(Command::Describe(check)) => describe(&check),
        Err(why) => {
            eprintln!("{why}");
            INVALID
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{Check, Command, parse};
    use std::path::PathBuf;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn check_takes_its_flags_and_defaults_to_the_current_directory() {
        assert_eq!(
            parse(&args(&["check"])),
            Ok(Command::Check(Check {
                root: PathBuf::from("."),
                policies: None,
                pull_request: false,
                json: false,
            }))
        );
        assert_eq!(
            parse(&args(&[
                "check",
                "--root",
                "r",
                "--policies",
                "p",
                "--pull-request",
                "--json"
            ])),
            Ok(Command::Check(Check {
                root: PathBuf::from("r"),
                policies: Some(PathBuf::from("p")),
                pull_request: true,
                json: true,
            }))
        );
        assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(
            parse(&args(&["describe", "--policies", "p"])),
            Ok(Command::Describe(Check {
                root: PathBuf::from("."),
                policies: Some(PathBuf::from("p")),
                pull_request: false,
                json: false,
            }))
        );
        assert!(
            parse(&args(&["describe", "--json"])).is_err(),
            "describe is always JSON"
        );
    }

    #[test]
    fn anything_else_is_refused_with_the_usage() {
        for bad in [&[][..], &["judge"], &["check", "--root"], &["check", "--x"]] {
            let why = parse(&args(bad)).expect_err("refused");
            assert!(why.contains("usage: pigeon-sentinel"), "{why}");
        }
    }
}
