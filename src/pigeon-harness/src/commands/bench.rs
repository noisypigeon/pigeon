//! `pigeon session bench`: push the working tree to the scratch ref and run on a bench; see
//! `.claude/decisions/2026-11-16-development-session-bench-containers.md`.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use agent::core::bench::deal::{Check, deal};
use agent::core::bench::{report, scratch_ref, session_id};
use serde_json::{Value, json};

use super::session::{agent, home, host};
use crate::cli::BenchCommand;

/// Longest the CLI waits for bench capacity, in seconds.
const CAPACITY: u64 = 600;
/// Exit code when the bench pool is unavailable.
const UNAVAILABLE: i32 = 75;

/// A start failure; `pool` is set when capacity never came.
struct Refused {
    why: String,
    pool: bool,
}

/// Runs a `pigeon session bench` command and returns its exit code.
#[must_use]
pub fn run(command: &BenchCommand) -> i32 {
    match command {
        BenchCommand::Push => match push() {
            Ok(commit) => {
                println!("{commit}");
                0
            }
            Err(e) => fail(&e),
        },
        BenchCommand::Run {
            bench,
            timeout,
            argv,
        } => run_one(*bench, argv, *timeout),
        BenchCommand::Ci { benches } => ci(*benches),
        BenchCommand::Status { run, bench, offset } => show(
            "bench.status",
            &json!({"run": run, "bench": bench, "offset": offset}),
        ),
        BenchCommand::Cancel { run, bench } => {
            show("bench.cancel", &json!({"run": run, "bench": bench}))
        }
        BenchCommand::Reset { bench, hard } => {
            show("bench.reset", &json!({"bench": bench, "hard": hard}))
        }
        BenchCommand::List => show("bench.list", &json!({})),
        BenchCommand::Warm { bench } => show("bench.warm", &json!({"bench": bench})),
    }
}

fn fail(e: &str) -> i32 {
    eprintln!("{e}");
    1
}

fn show(operation: &str, args: &Value) -> i32 {
    match op(operation, args) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            0
        }
        Err(e) => fail(&e),
    }
}

fn session() -> Result<(String, String), String> {
    let none = || "pigeon: no session ticket in ~/.config/pigeon/session.json".to_string();
    let path = home().ok_or_else(none)?.join(".config/pigeon/session.json");
    let v: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(none)?;
    match (v["label"].as_str(), v["ticket"].as_str()) {
        (Some(l), Some(t)) => Ok((l.to_string(), t.to_string())),
        _ => Err(none()),
    }
}

fn op(operation: &str, args: &Value) -> Result<Value, String> {
    let (_, ticket) = session()?;
    let mut args = args.clone();
    args["ticket"] = json!(ticket);
    let seconds = if operation == "bench.status" { 40 } else { 60 };
    let url = format!("https://{}/v0/run", host());
    let mut response = agent(seconds)
        .post(&url)
        .header("content-type", "application/json")
        .send(json!({"operation": operation, "args": args}).to_string())
        .map_err(|e| e.to_string())?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let body: Value = serde_json::from_str(&text).map_err(|_| text.clone())?;
    if body["ok"].as_bool() == Some(true) {
        return Ok(body["result"].clone());
    }
    Err(body["violations"][0]
        .as_str()
        .map(str::to_string)
        .or_else(|| body["result"].as_str().map(str::to_string))
        .unwrap_or(text))
}

fn git(args: &[&str]) -> Result<String, String> {
    git_env(args, &[], None)
}

fn git_env(args: &[&str], env: &[(&str, &str)], dir: Option<&str>) -> Result<String, String> {
    let mut c = Command::new("git");
    c.args(args).stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Snapshots the working tree without touching the index, pushes it, and returns its commit.
fn push() -> Result<String, String> {
    let (label, ticket) = session()?;
    let top = git(&["rev-parse", "--show-toplevel"])?;
    let gitdir = git(&["rev-parse", "--absolute-git-dir"])?;
    let index = format!("{gitdir}/pigeon-bench-index");
    let env = [("GIT_INDEX_FILE", index.as_str())];
    git_env(&["read-tree", "HEAD"], &env, None)?;
    git_env(&["add", "-A"], &env, Some(&top))?;
    let tree = git_env(&["write-tree"], &env, None)?;
    let cache = format!("{gitdir}/pigeon-bench-pushed");
    if let Ok(text) = std::fs::read_to_string(&cache)
        && let Some((t, c)) = text.trim().split_once(' ')
        && t == tree
    {
        return Ok(c.to_string());
    }
    let commit = git_env(
        &["commit-tree", &tree, "-p", "HEAD", "-m", "bench snapshot"],
        &[
            ("GIT_AUTHOR_NAME", "pigeon bench"),
            ("GIT_COMMITTER_NAME", "pigeon bench"),
            ("GIT_AUTHOR_EMAIL", "bench@pigeon.invalid"),
            ("GIT_COMMITTER_EMAIL", "bench@pigeon.invalid"),
        ],
        None,
    )?;
    let id = session_id(&label)
        .ok_or_else(|| "pigeon: this session's label has no session ID".to_string())?;
    let remote = git(&["remote", "get-url", "artifacts"])
        .unwrap_or_else(|_| format!("https://{}/git/pigeon/pigeon.git", host()));
    let header = format!("http.extraHeader=X-Pigeon-Ticket: {ticket}");
    let refspec = format!("{commit}:{}", scratch_ref(id));
    let mut last = String::new();
    for delay in [0, 5, 15] {
        sleep(Duration::from_secs(delay));
        match git(&[
            "-c",
            "http.postBuffer=524288000",
            "-c",
            &header,
            "push",
            "--force",
            "-q",
            &remote,
            &refspec,
        ]) {
            Ok(_) => {
                std::fs::write(&cache, format!("{tree} {commit}")).map_err(|e| e.to_string())?;
                return Ok(commit);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Streams a run's log (to stdout, or into `sink`) and returns its exit code.
fn follow(run: &str, bench: u8, mut sink: Option<&mut String>) -> i32 {
    let started = Instant::now();
    let mut offset = 0;
    loop {
        let r = match op(
            "bench.status",
            &json!({"run": run, "bench": bench, "offset": offset, "wait": 20}),
        ) {
            Ok(r) => r,
            Err(e) => return fail(&e),
        };
        let log = r["log"].as_str().unwrap_or("");
        if let Some(s) = sink.as_deref_mut() {
            s.push_str(log);
        } else {
            print!("{log}");
            let _ = std::io::stdout().flush();
        }
        offset = r["next"].as_u64().unwrap_or(offset);
        let state = r["state"].as_str().unwrap_or("");
        if report::done(state) {
            if sink.is_none() {
                eprintln!(
                    "\npigeon: run {run} {state} in {}s",
                    started.elapsed().as_secs()
                );
            }
            return report::exit_code(state, r["exit"].as_i64());
        }
    }
}

/// Pushes and starts `argv` on a bench, waiting for capacity; the run ID.
fn start(
    bench: u8,
    argv: &[String],
    timeout: Option<u32>,
    sha: Option<&str>,
) -> Result<String, Refused> {
    let sha = match sha {
        Some(s) => s.to_string(),
        None => push().map_err(|why| Refused { why, pool: false })?,
    };
    let began = Instant::now();
    loop {
        let mut call = json!({"sha": sha, "argv": argv, "bench": bench});
        if let Some(t) = timeout {
            call["timeout"] = json!(t);
        }
        match op("bench.run", &call) {
            Ok(r) => return Ok(r["run"].as_str().unwrap_or_default().to_string()),
            Err(why) if report::waiting(&why) => {
                if began.elapsed().as_secs() >= CAPACITY {
                    return Err(Refused {
                        why: format!(
                            "pigeon: {why}; the bench pool is unavailable: run only `cargo fmt --all -- --check` here, push, and let the gate check"
                        ),
                        pool: true,
                    });
                }
                eprintln!("{why}");
                sleep(Duration::from_secs(30));
            }
            Err(why) => return Err(Refused { why, pool: false }),
        }
    }
}

fn refused(r: &Refused) -> i32 {
    eprintln!("{}", r.why);
    if r.pool { UNAVAILABLE } else { 1 }
}

fn run_one(bench: u8, argv: &[String], timeout: Option<u32>) -> i32 {
    match start(bench, argv, timeout, None) {
        Ok(run) => follow(&run, bench, None),
        Err(r) => refused(&r),
    }
}

/// `ci`'s one-bench command.
fn ci_scoped_argv() -> Vec<String> {
    ["pigeon", "dev", "ci-scoped"].map(String::from).to_vec()
}

/// `ci`'s first step: the checks this change needs, as JSON.
fn scope_argv() -> Vec<String> {
    [
        "pigeon",
        "dev",
        "scope",
        "--base",
        "artifacts/main",
        "--json",
    ]
    .map(String::from)
    .to_vec()
}

/// One of `ci`'s checks, by name.
fn check_argv(name: &str) -> Vec<String> {
    ["bench", "check", name].map(String::from).to_vec()
}

fn ci(benches: u8) -> i32 {
    let n = benches.clamp(1, 3);
    if n == 1 {
        return run_one(0, &ci_scoped_argv(), None);
    }
    let scope = scope_argv();
    let mut log = String::new();
    let sha = match push() {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let code = match start(0, &scope, None, Some(&sha)) {
        Ok(run) => follow(&run, 0, Some(&mut log)),
        Err(r) => return refused(&r),
    };
    if code != 0 {
        print!("{log}");
        return code;
    }
    let checks = match report::checks_in(&log) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    if checks.is_empty() {
        println!("pigeon: this change needs no checks");
        return 0;
    }
    let results: Vec<(String, bool, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = deal(&checks, n)
            .into_iter()
            .enumerate()
            .filter(|(_, list)| !list.is_empty())
            .map(|(k, list)| {
                let sha = sha.as_str();
                s.spawn(move || lane(u8::try_from(k).unwrap_or(0), &list, sha))
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (name, ok, out) in &results {
        if !ok {
            println!("== {name}\n{out}");
        }
    }
    i32::from(results.iter().any(|(_, ok, _)| !ok))
}

/// Runs a bench's checks in order: each check's name, whether it passed, and its log.
fn lane(k: u8, list: &[Check], sha: &str) -> Vec<(String, bool, String)> {
    let mut done = Vec::new();
    for check in list {
        let began = Instant::now();
        let argv = check_argv(&check.name);
        let mut out = String::new();
        let ok = match start(k, &argv, None, Some(sha)) {
            Ok(run) => follow(&run, k, Some(&mut out)) == 0,
            Err(r) => {
                out = r.why;
                false
            }
        };
        let verdict = if ok { "PASS" } else { "FAIL" };
        println!(
            "{verdict} {} ({}s, bench {k})",
            check.name,
            began.elapsed().as_secs()
        );
        done.push((check.name.clone(), ok, out));
    }
    done
}

#[cfg(test)]
mod tests {
    use agent::core::bench::allow::allow;

    #[test]
    fn every_command_ci_sends_is_allowed() {
        for argv in [
            super::ci_scoped_argv(),
            super::scope_argv(),
            super::check_argv("clippy"),
        ] {
            assert!(allow(&argv).is_ok(), "{argv:?}");
        }
    }
}
