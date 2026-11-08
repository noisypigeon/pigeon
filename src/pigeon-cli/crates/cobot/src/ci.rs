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

/// The file at `path` on the checkout's fetched `base`, if it exists there.
#[must_use]
pub fn base_file(dir: &Path, base: &str, path: &str) -> Option<String> {
    git(
        dir,
        "",
        "",
        &["show", &format!("refs/remotes/origin/{base}:{path}")],
    )
    .ok()
}

/// Whether `path` existed on the fetched `base` and `sha` only adds lines to it.
#[must_use]
pub fn appends_only(dir: &Path, base: &str, sha: &str, path: &str) -> bool {
    let base = format!("refs/remotes/origin/{base}");
    let existed = git(dir, "", "", &["cat-file", "-e", &format!("{base}:{path}")]).is_ok();
    let range = format!("{base}...{sha}");
    existed
        && git(dir, "", "", &["diff", "--numstat", &range, "--", path])
            .is_ok_and(|n| n.split_whitespace().nth(1) == Some("0"))
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

/// The commit on `remote`'s `base` that merged `sha`, if one did: the newest whose message
/// has the trailer `Change-Head: <sha>`, which only the merge job writes.
///
/// # Errors
///
/// [`Error::Git`] when git fails.
pub fn merged_already(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    sha: &str,
) -> Result<Option<String>, Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    git(dir, remote, token, &["init", "-q"])?;
    let base_ref = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    git(
        dir,
        remote,
        token,
        &["fetch", "-q", "--no-tags", remote, &base_ref],
    )?;
    let grep = format!("^Change-Head: {sha}$");
    let found = git(
        dir,
        "",
        "",
        &[
            "log",
            "-n",
            "1",
            "--format=%H",
            "--grep",
            &grep,
            &format!("refs/remotes/origin/{base}"),
        ],
    )?;
    Ok(Some(found).filter(|f| !f.is_empty()))
}

/// How a change's head stands to its base branch after the branch moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// The base is still its base.
    Current,
    /// None of its files changed on the base.
    Disjoint,
    /// Some did, and git merges them without conflict.
    Clean,
    /// Git can't merge them.
    Conflicting,
}

/// A restack: how the head stood, and the head it became, if one was pushed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Restack {
    /// How it stood.
    pub relation: Relation,
    /// The head it started from.
    pub from: String,
    /// The base it was replayed onto.
    pub onto: String,
    /// The new head pushed to its branch; none when current or conflicting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Whether the change's own diff is the same, by `git patch-id --stable`.
    #[serde(default)]
    pub same_patch: bool,
    /// The paths git couldn't merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// For a conflict: each path's changes on both sides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<Conflict>,
    /// For a conflict: the messages of the base's commits that touched the paths, which name
    /// the changes merged and the ADRs they cite.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub messages: String,
}

/// One conflicting path's changes on both sides, each diff cut to [`CONTEXT_MAX`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    /// The path.
    pub path: String,
    /// The subjects of the base's commits that touched it since the fork.
    #[serde(default)]
    pub commits: Vec<String>,
    /// The base's diff of it since the fork.
    #[serde(default)]
    pub base_diff: String,
    /// The change's diff of it since the fork.
    #[serde(default)]
    pub change_diff: String,
}

/// The most of each diff, and of the messages, a conflict's context keeps.
pub const CONTEXT_MAX: usize = 4096;

/// `text` cut to at most `max` bytes, at a character boundary.
fn cut(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// The context of `paths`, in conflict between `fork..onto` and `fork..sha`.
fn conflict_context(
    dir: &Path,
    fork: &str,
    onto: &str,
    sha: &str,
    paths: &[String],
) -> Result<(Vec<Conflict>, String), Error> {
    let range = format!("{fork}..{onto}");
    let mut context = Vec::new();
    for path in paths {
        let commits = git(dir, "", "", &["log", "--format=%s", &range, "--", path])?;
        context.push(Conflict {
            path: path.clone(),
            commits: commits.lines().map(str::to_string).collect(),
            base_diff: cut(
                &git(dir, "", "", &["diff", fork, onto, "--", path])?,
                CONTEXT_MAX,
            ),
            change_diff: cut(
                &git(dir, "", "", &["diff", fork, sha, "--", path])?,
                CONTEXT_MAX,
            ),
        });
    }
    let mut args = vec!["log", "--format=%B", range.as_str(), "--"];
    args.extend(paths.iter().map(String::as_str));
    let messages = cut(&git(dir, "", "", &args)?, CONTEXT_MAX);
    Ok((context, messages))
}

fn patch_id(dir: &Path, from: &str, to: &str) -> Result<String, Error> {
    let diff = command("", "")
        .arg("-C")
        .arg(dir)
        .args(["diff", "--binary", from, to])
        .output()
        .map_err(|e| Error::Git {
            args: "diff".into(),
            message: e.to_string(),
        })?;
    let mut child = command("", "")
        .arg("-C")
        .arg(dir)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Git {
            args: "patch-id".into(),
            message: e.to_string(),
        })?;
    if let Some(mut i) = child.stdin.take() {
        use std::io::Write as _;
        i.write_all(&diff.stdout).map_err(|e| Error::Git {
            args: "patch-id".into(),
            message: e.to_string(),
        })?;
    }
    let out = child.wait_with_output().map_err(|e| Error::Git {
        args: "patch-id".into(),
        message: e.to_string(),
    })?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}

/// `ours` and `theirs` merged in `dir`'s work tree: the tree, or the paths in conflict.
fn merge_tree(dir: &Path, ours: &str, theirs: &str) -> Result<Result<String, Vec<String>>, Error> {
    let out = command("", "")
        .arg("-C")
        .arg(dir)
        .args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            ours,
            theirs,
        ])
        .output()
        .map_err(|e| Error::Git {
            args: "merge-tree".into(),
            message: e.to_string(),
        })?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let tree = lines.next().unwrap_or_default().to_string();
    match out.status.code() {
        Some(0) => Ok(Ok(tree)),
        Some(1) => Ok(Err(lines.map(str::to_string).collect())),
        _ => Err(Error::Git {
            args: "merge-tree".into(),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
    }
}

/// A commit of `tree` on `parent` with `message`, by pigeon.
fn commit_tree(dir: &Path, tree: &str, parent: &str, message: &str) -> Result<String, Error> {
    let commit = command("", "")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=pigeon",
            "-c",
            "user.email=pigeon@users.noreply.pigeon.dev",
        ])
        .args(["commit-tree", tree, "-p", parent, "-F", "-"])
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
    Ok(String::from_utf8_lossy(&commit.stdout).trim().to_string())
}

/// Replays `sha`, `branch`'s head checked out in `dir` by [`checkout`], onto `base` as it now
/// stands, as one commit carrying the message of the change's first commit, and pushes it to
/// `branch` only if `sha` is still its head. Paths matching a `union` glob merge by union; see
/// `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.
///
/// # Errors
///
/// [`Error::Git`] when git fails, or the push loses to a newer head.
pub fn restack(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    branch: &str,
    sha: &str,
    union: &[&str],
) -> Result<Restack, Error> {
    let attributes = union.iter().fold(String::new(), |mut a, g| {
        let _ = writeln!(a, "{g} merge=union");
        a
    });
    let info = dir.join(".git/info");
    std::fs::create_dir_all(&info)
        .and_then(|()| std::fs::write(info.join("attributes"), attributes))
        .map_err(|e| Error::Io {
            path: info.clone(),
            message: e.to_string(),
        })?;
    let onto = git(
        dir,
        "",
        "",
        &["rev-parse", &format!("refs/remotes/origin/{base}")],
    )?;
    let fork = git(dir, "", "", &["merge-base", &onto, sha])?;
    let mut r = Restack {
        relation: Relation::Current,
        from: sha.to_string(),
        onto: onto.clone(),
        to: None,
        same_patch: true,
        conflicts: Vec::new(),
        context: Vec::new(),
        messages: String::new(),
    };
    if fork == onto {
        return Ok(r);
    }
    let mine = changed_since(dir, &fork, sha)?;
    let theirs = changed_since(dir, &fork, &onto)?;
    let detached = ["-c", "advice.detachedHead=false", "checkout", "-q", &onto];
    git(dir, "", "", &detached)?;
    let tree = match merge_tree(dir, &onto, sha)? {
        Ok(tree) => tree,
        Err(conflicts) => {
            (r.context, r.messages) = conflict_context(dir, &fork, &onto, sha, &conflicts)?;
            r.relation = Relation::Conflicting;
            r.same_patch = false;
            r.conflicts = conflicts;
            return Ok(r);
        }
    };
    let commits = git(
        dir,
        "",
        "",
        &["rev-list", "--reverse", &format!("{fork}..{sha}")],
    )?;
    let first = commits.lines().next().unwrap_or(sha);
    let message = git(dir, "", "", &["log", "-1", "--format=%B", first])?;
    let new = commit_tree(dir, &tree, &onto, &format!("{message}\n"))?;
    r.relation = if mine.iter().any(|p| theirs.contains(p)) {
        Relation::Clean
    } else {
        Relation::Disjoint
    };
    r.same_patch = patch_id(dir, &fork, sha)? == patch_id(dir, &onto, &new)?;
    let lease = format!("--force-with-lease=refs/heads/{branch}:{sha}");
    let refspec = format!("{new}:refs/heads/{branch}");
    git(
        dir,
        remote,
        token,
        &["push", "-q", &lease, remote, &refspec],
    )?;
    r.to = Some(new);
    Ok(r)
}
