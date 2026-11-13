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

const USAGE: &str = "usage: pigeon-sentinel check [--root DIR] [--policies DIR] [--pull-request] [--json]\n       pigeon-sentinel describe [--root DIR] [--policies DIR]\n       pigeon-sentinel evict [--root DIR] [--policies DIR] --candidate JSON\n       pigeon-sentinel --version";

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
    /// Which eviction policy grants evicting a candidate, given as a JSON object.
    Evict(Check, String),
    Version,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let (describing, evicting) = match args.first().map(String::as_str) {
        Some("--version" | "-V") => return Ok(Command::Version),
        Some("check") => (false, false),
        Some("describe") => (true, false),
        Some("evict") => (true, true),
        _ => return Err(USAGE.into()),
    };
    let mut candidate = None;
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
            "--candidate" if evicting => {
                candidate = Some(value("--candidate")?.display().to_string());
            }
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(match (evicting, describing) {
        (true, _) => Command::Evict(
            check,
            candidate.ok_or_else(|| format!("evict needs --candidate\n{USAGE}"))?,
        ),
        (false, true) => Command::Describe(check),
        (false, false) => Command::Check(check),
    })
}

/// `evict`: each eviction policy's verdict on the candidate, and the one that grants it.
fn evict(check: &Check, candidate: &str) -> u8 {
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
    let fields: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(candidate) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("sentinel: --candidate must be a JSON object: {e}");
            return INVALID;
        }
    };
    let candidate: sentinel::Candidate = fields
        .into_iter()
        .map(|(k, v)| {
            let text = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            (k, text)
        })
        .collect();
    let (results, granted) = sentinel::evict(&policies, &candidate);
    println!(
        "{}",
        serde_json::json!({ "granted": granted, "results": results })
    );
    if granted.is_some() {
        SUCCESS
    } else {
        VIOLATION
    }
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
        Ok(Command::Evict(check, candidate)) => evict(&check, &candidate),
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
    fn evict_takes_a_candidate_and_needs_one() {
        assert_eq!(
            parse(&args(&["evict", "--candidate", "{}"])),
            Ok(Command::Evict(
                Check {
                    root: PathBuf::from("."),
                    policies: None,
                    pull_request: false,
                    json: false,
                },
                "{}".into()
            ))
        );
        assert!(parse(&args(&["evict"])).is_err());
        assert!(parse(&args(&["check", "--candidate", "{}"])).is_err());
    }

    #[test]
    fn anything_else_is_refused_with_the_usage() {
        for bad in [&[][..], &["judge"], &["check", "--root"], &["check", "--x"]] {
            let why = parse(&args(bad)).expect_err("refused");
            assert!(why.contains("usage: pigeon-sentinel"), "{why}");
        }
    }
}
