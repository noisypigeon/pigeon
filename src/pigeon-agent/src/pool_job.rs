//! The pool job: a brief carried out by `claude -p` on a capacity pool, in a user-namespace sandbox, then its checks, commit and push; see `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md` §7 and its 2026-11-17 amendments.

use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::checks::ci::{self, Landed};
use crate::control::{JobSpec, Outcome, Report};
use crate::core::pool::job as pj;
use crate::jobs;

fn failed(spec: &JobSpec, why: &str) -> Report {
    Report {
        violations: vec![why.to_string()],
        pool: Some(pj::Pooled {
            outcome: pj::Outcome::Failed,
            why: why.to_string(),
            turns: 0,
            harness_cost: None,
            checks: Vec::new(),
            escalation: None,
            pushed: None,
        }),
        ..jobs::report(spec, Outcome::Failed, why)
    }
}

fn write(path: &Path, text: &str, mode: u32) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

fn make(dir: &Path, mode: u32) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
}

fn setup(work: &Path, job: &pj::Job) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(work);
    make(work, 0o755)?;
    for d in ["home", "cargo", "out"] {
        make(&work.join(d), 0o1777)?;
    }
    let path = |p: &str| work.join(p).to_string_lossy().into_owned();
    let brief = serde_json::to_string_pretty(&job.brief).map_err(std::io::Error::other)?;
    write(&work.join("brief.json"), &brief, 0o644)?;
    write(
        &work.join("home/.gitconfig"),
        "[safe]\n\tdirectory = *\n",
        0o644,
    )?;
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let mcp = pj::mcp_config(&exe, &path("brief.json"), &path("out/escalation.json"));
    write(&work.join("mcp.json"), &mcp, 0o644)
}

/// Opens the checkout to the sandbox's uid; `.git` stays the runner's.
fn open(dir: &Path) -> std::io::Result<()> {
    walk(dir, true, false)
}

fn walk(dir: &Path, root: bool, in_git: bool) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let git = in_git || (root && entry.file_name() == ".git");
        let mode = meta.permissions().mode();
        let new = if git {
            mode & !0o022
        } else if meta.is_dir() {
            0o777
        } else if mode & 0o100 != 0 {
            mode | 0o777
        } else {
            mode | 0o666
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(new))?;
        if meta.is_dir() {
            walk(&path, false, git)?;
        }
    }
    Ok(())
}

/// Runs the pool job `spec` in `dir`.
#[must_use]
pub fn run(spec: &JobSpec, dir: &Path) -> Report {
    let job = match pj::parse(&spec.env) {
        Ok(j) => j,
        Err(why) => return failed(spec, &why),
    };
    if let Err(e) = ci::checkout(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
    ) {
        return failed(spec, &e.to_string());
    }
    let work = std::env::temp_dir().join(format!("pool-{}", spec.id));
    if let Err(e) = setup(&work, &job).and_then(|()| open(dir)) {
        return failed(spec, &format!("the sandbox did not set up: {e}"));
    }
    let path = |p: &str| work.join(p).to_string_lossy().into_owned();
    let (home, cargo, mcp) = (path("home"), path("cargo"), path("mcp.json"));
    let argv = pj::sandboxed(&pj::claude_env(&job, &home), &pj::claude_argv(&job, &mcp));
    let limit = Duration::from_secs(u64::from(job.minutes) * 60);
    let started = Instant::now();
    let (_, log) = ci::exec(dir, &argv, limit, &spec.token);
    let timed_out = started.elapsed() >= limit;
    let mut log = log
        .replace(&job.token, "…")
        .replace(&job.access_secret, "…");
    let escalation = std::fs::read_to_string(work.join("out/escalation.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let escalated = escalation.is_some();
    let mut checks: Vec<(String, bool)> = Vec::new();
    if !escalated && !timed_out {
        let listed = job.brief.get("checks").and_then(Value::as_array);
        for c in listed.into_iter().flatten().filter_map(Value::as_str) {
            if !pj::runnable(c) {
                continue;
            }
            let argv = pj::sandboxed(&pj::check_env(&home, &cargo), &pj::check_argv(c));
            let (ok, out) = ci::exec(dir, &argv, ci::TIMEOUT, &spec.token);
            checks.push((c.to_string(), ok));
            log.push_str(&out);
            if !ok {
                break;
            }
        }
    }
    let checks_ok = checks.iter().all(|(_, ok)| *ok);
    let s = pj::stream(&log);
    let (outcome, mut why) = pj::outcome(escalated, timed_out, &s, checks_ok);
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .is_ok_and(|o| !o.stdout.is_empty());
    let mut pushed = None;
    if dirty {
        let msg = pj::commit_message(&job, &spec.scope, outcome != pj::Outcome::Done);
        match ci::land(
            dir,
            &spec.remote,
            &spec.token,
            &spec.branch,
            &msg,
            None,
            &[],
        ) {
            Ok(Landed::Pushed(c)) => pushed = Some(c),
            Ok(Landed::Moved) => why.push_str("; the branch moved, nothing pushed"),
            Err(e) => {
                let _ = write!(why, "; push failed: {e}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    let top = if outcome == pj::Outcome::Done {
        Outcome::Passed
    } else {
        Outcome::Failed
    };
    Report {
        pool: Some(pj::Pooled {
            outcome,
            why,
            turns: s.turns,
            harness_cost: s.cost,
            checks,
            escalation,
            pushed,
        }),
        ..jobs::report(spec, top, &log)
    }
}

/// Serves `brief_show` and `pool_escalate` as MCP on stdio; an escalation goes to `escalation`.
#[must_use]
pub fn tools(brief: &Path, escalation: &Path) -> i32 {
    let brief = match std::fs::read_to_string(brief) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read the brief: {e}");
            return 1;
        }
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let (reply, esc) = pj::answer(&line, &brief, crate::VERSION);
        if let Some(r) = reply {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
        if let Some(e) = esc {
            let _ = std::fs::write(escalation, e.to_string());
        }
    }
    0
}
