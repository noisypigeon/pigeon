//! A job's checkout and commands, in a job Container; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4. The token
//! reaches git only through the environment, and is cut from every output.

use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::Error;
use crate::git::command;

/// The longest a job's command runs before it is killed.
pub const TIMEOUT: Duration = Duration::from_mins(45);

fn scrub(text: &str, token: &str) -> String {
    if token.len() >= 8 {
        text.replace(token, "…")
    } else {
        text.to_string()
    }
}

fn git(dir: &Path, remote: &str, token: &str, args: &[&str]) -> Result<String, Error> {
    let out = command(remote, token)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| Error::Git {
            args: args.join(" "),
            message: e.to_string(),
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(Error::Git {
            args: scrub(&args.join(" "), token),
            message: scrub(String::from_utf8_lossy(&out.stderr).trim(), token),
        })
    }
}

/// Fetches `branch` and `base` from `remote` into a new repository at `dir`, and checks out
/// `sha`, which must be `branch`'s head.
///
/// # Errors
///
/// [`Error::Git`] when git fails, or `sha` isn't the branch's head.
pub fn checkout(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    branch: &str,
    sha: &str,
) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    git(dir, remote, token, &["init", "-q"])?;
    let base_ref = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    let branch_ref = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    git(
        dir,
        remote,
        token,
        &["fetch", "-q", "--no-tags", remote, &base_ref, &branch_ref],
    )?;
    let head = git(
        dir,
        remote,
        token,
        &["rev-parse", &format!("refs/remotes/origin/{branch}")],
    )?;
    if head != sha {
        return Err(Error::Git {
            args: format!("rev-parse {branch}"),
            message: format!("{branch}'s head is {head}, not {sha}; push again or report its head"),
        });
    }
    git(
        dir,
        remote,
        token,
        &["-c", "advice.detachedHead=false", "checkout", "-q", sha],
    )?;
    Ok(())
}

/// The paths `sha` changes since it left `base`, as `git diff --name-only base...sha`.
///
/// # Errors
///
/// [`Error::Git`] when git fails.
pub fn changed(dir: &Path, base: &str, sha: &str) -> Result<Vec<String>, Error> {
    let range = format!("refs/remotes/origin/{base}...{sha}");
    let out = git(dir, "", "", &["diff", "--name-only", &range])?;
    Ok(out
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect())
}

/// Runs `argv` in `dir` for at most `timeout`: whether it passed, and its output, stdout then
/// stderr, with `secret` cut out.
#[must_use]
pub fn exec(dir: &Path, argv: &[String], timeout: Duration, secret: &str) -> (bool, String) {
    let Some((program, rest)) = argv.split_first() else {
        return (false, "an empty command".into());
    };
    let child = Command::new(program)
        .args(rest)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (false, format!("{program} did not start: {e}")),
    };
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let readers = [
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_string(&mut s);
            }
            s
        }),
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut s);
            }
            s
        }),
    ];
    let started = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let mut log = String::new();
    for r in readers {
        log.push_str(&r.join().unwrap_or_default());
    }
    if !ok && started.elapsed() >= timeout {
        let _ = writeln!(
            log,
            "
killed after {} minutes",
            timeout.as_secs() / 60
        );
    }
    (ok, scrub(&log, secret))
}
