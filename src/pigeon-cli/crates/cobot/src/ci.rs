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
    let mut args = vec!["fetch", "-q", "--no-tags", remote, base_ref.as_str()];
    if branch != base {
        args.push(branch_ref.as_str());
    }
    git(dir, remote, token, &args)?;
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

/// Commits `sha`'s tree onto `base`'s head as one commit with `message`, and pushes it to
/// `remote`'s `base`, then deletes `branch` there; the commit pushed. `base`'s head must be an
/// ancestor of `sha`, so the commit's tree is exactly the checked one; the push is never
/// forced, so a `base` that moved since the fetch is refused.
///
/// # Errors
///
/// [`Error::Git`] when `base` isn't an ancestor of `sha`, or git fails.
pub fn squash_onto(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    branch: &str,
    sha: &str,
    message: &str,
) -> Result<String, Error> {
    let on = format!("refs/remotes/origin/{base}");
    let ancestor = command("", "")
        .arg("-C")
        .arg(dir)
        .args(["merge-base", "--is-ancestor", &on, sha])
        .status()
        .map_err(|e| Error::Git {
            args: "merge-base".into(),
            message: e.to_string(),
        })?;
    if !ancestor.success() {
        return Err(Error::Git {
            args: "merge-base --is-ancestor".into(),
            message: format!("{base} has moved past {branch}'s base; rebase onto {base} and push"),
        });
    }
    let parent = git(dir, "", "", &["rev-parse", &on])?;
    let tree = format!("{sha}^{{tree}}");
    let commit = command("", "")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=pigeon",
            "-c",
            "user.email=pigeon@users.noreply.pigeon.dev",
        ])
        .args(["commit-tree", &tree, "-p", &parent, "-F", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            if let Some(mut i) = c.stdin.take() {
                use std::io::Write as _;
                i.write_all(message.as_bytes())?;
            }
            c.wait_with_output()
        })
        .map_err(|e| Error::Git {
            args: "commit-tree".into(),
            message: e.to_string(),
        })?;
    if !commit.status.success() {
        return Err(Error::Git {
            args: "commit-tree".into(),
            message: String::from_utf8_lossy(&commit.stderr).trim().to_string(),
        });
    }
    let new = String::from_utf8_lossy(&commit.stdout).trim().to_string();
    git(
        dir,
        remote,
        token,
        &["push", "-q", remote, &format!("{new}:refs/heads/{base}")],
    )?;
    git(
        dir,
        remote,
        token,
        &["push", "-q", remote, &format!(":refs/heads/{branch}")],
    )?;
    Ok(new)
}

/// Fetches every tag from `remote` into `dir`, as a site's template is read from its tag.
///
/// # Errors
///
/// [`Error::Git`] when git fails.
pub fn fetch_tags(dir: &Path, remote: &str, token: &str) -> Result<(), Error> {
    git(
        dir,
        remote,
        token,
        &["fetch", "-q", remote, "+refs/tags/*:refs/tags/*"],
    )
    .map(|_| ())
}

/// The paths changed from `from` to `to`, both in `dir`.
///
/// # Errors
///
/// [`Error::Git`] when either is unknown there.
pub fn changed_since(dir: &Path, from: &str, to: &str) -> Result<Vec<String>, Error> {
    let out = git(dir, "", "", &["diff", "--name-only", from, to])?;
    Ok(out
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect())
}
