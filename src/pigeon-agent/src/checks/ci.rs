//! A job's checkout and commands, in a job Container; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4. The token
//! reaches git only through the environment, and is cut from every output.

use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::core::change::{STALE_BASE, TRANSIENT_PUSH, lockfile, push};
use crate::git::command;
use crate::infra::Error;

/// The longest a job's command runs before it is killed.
pub const TIMEOUT: Duration = Duration::from_mins(45);

fn scrub(text: &str, token: &str) -> String {
    if token.len() >= 8 {
        text.replace(token, "…")
    } else {
        text.to_string()
    }
}

/// Runs git in `dir`; a fetch refused for now is retried after each of `push::BACKOFF`.
fn git(dir: &Path, remote: &str, token: &str, args: &[&str]) -> Result<String, Error> {
    if args.first() != Some(&"fetch") {
        return git_once(dir, remote, token, args);
    }
    let mut waits = push::BACKOFF.iter();
    loop {
        match git_once(dir, remote, token, args) {
            Err(Error::Git { message, .. }) if push::transient(&message) => match waits.next() {
                Some(&s) => std::thread::sleep(Duration::from_secs(s)),
                None => {
                    return Err(Error::Git {
                        args: "fetch".into(),
                        message: format!(
                            "{TRANSIENT_PUSH} {} times: {message}",
                            push::BACKOFF.len() + 1
                        ),
                    });
                }
            },
            other => return other,
        }
    }
}

fn git_once(dir: &Path, remote: &str, token: &str, args: &[&str]) -> Result<String, Error> {
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

/// Fetches `branch` from `remote` into a new repository at `dir`, and checks out `pinned`,
/// which must be on the branch, or else the branch's head as fetched: the commit checked out.
///
/// # Errors
///
/// [`Error::Git`] when git fails, or `pinned` isn't on the branch.
pub fn checkout_on(
    dir: &Path,
    remote: &str,
    token: &str,
    branch: &str,
    pinned: Option<&str>,
) -> Result<String, Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    git(dir, remote, token, &["init", "-q"])?;
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    git(
        dir,
        remote,
        token,
        &["fetch", "-q", "--no-tags", remote, &spec],
    )?;
    let head = git(
        dir,
        remote,
        token,
        &["rev-parse", &format!("refs/remotes/origin/{branch}")],
    )?;
    let sha = pinned.unwrap_or(&head).to_string();
    if sha != head && git(dir, "", "", &["merge-base", "--is-ancestor", &sha, &head]).is_err() {
        return Err(Error::Git {
            args: format!("merge-base --is-ancestor {sha} {branch}"),
            message: format!("{sha} is not on {branch}"),
        });
    }
    git(
        dir,
        remote,
        token,
        &["-c", "advice.detachedHead=false", "checkout", "-q", &sha],
    )?;
    let out = git(dir, "", "", &["rev-parse", "HEAD"])?;
    if out != sha {
        return Err(Error::Git {
            args: "rev-parse HEAD".into(),
            message: format!("checked out {out}, not {sha}"),
        });
    }
    Ok(sha)
}

/// `branch`'s head on `remote` now, by `git ls-remote`, run in `dir`.
///
/// # Errors
///
/// [`Error::Git`] when git fails or `remote` has no such branch.
pub fn remote_head(dir: &Path, remote: &str, token: &str, branch: &str) -> Result<String, Error> {
    let refname = format!("refs/heads/{branch}");
    let out = git(dir, remote, token, &["ls-remote", remote, &refname])?;
    out.lines()
        .find_map(|l| l.split_once('\t').filter(|(_, r)| *r == refname))
        .map(|(sha, _)| sha.to_string())
        .ok_or_else(|| Error::Git {
            args: format!("ls-remote {refname}"),
            message: format!("no {refname}"),
        })
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

/// Fetches `branch` into the checkout as `refs/remotes/origin/<branch>`, for [`changed`].
///
/// # Errors
///
/// [`Error::Git`] when the fetch fails.
pub fn fetch_branch(dir: &Path, remote: &str, token: &str, branch: &str) -> Result<(), Error> {
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    git(dir, remote, token, &["fetch", "-q", remote, &spec]).map(|_| ())
}

/// The commit of fetched `branch` that `sha` is built on, their merge base: where a restack
/// starts once that branch has merged, even if it was restacked before merging; see the yield
/// ADR's 2026-11-14 amendment on `after`.
///
/// # Errors
///
/// When git fails.
pub fn built_on_head(dir: &Path, branch: &str, sha: &str) -> Result<String, Error> {
    let fetched = format!("refs/remotes/origin/{branch}");
    git(dir, "", "", &["merge-base", &fetched, sha])
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

/// What a program printed, and its exit code: none when a signal ended it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code.
    pub code: Option<i32>,
    /// Its standard output.
    pub stdout: String,
    /// Its standard error.
    pub stderr: String,
}

/// Runs `program` with `args` and only `env` added to the job's environment, input closed.
///
/// # Errors
///
/// Why it didn't start.
pub fn run_program(program: &Path, args: &[String], env: &[(&str, String)]) -> Result<Ran, String> {
    let out = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{} didn't run: {e}", program.display()))?;
    Ok(Ran {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Writes each file under `path` on the fetched `base` to `out`, at its path below `path`; how
/// many it wrote.
///
/// # Errors
///
/// [`Error::Git`] when git fails, [`Error::Io`] when a file can't be written.
pub fn base_tree(dir: &Path, base: &str, path: &str, out: &Path) -> Result<usize, Error> {
    let rev = format!("refs/remotes/origin/{base}");
    let prefix = path.trim_end_matches('/');
    let listed = git(
        dir,
        "",
        "",
        &["ls-tree", "-r", "--name-only", &rev, "--", prefix],
    )?;
    let io = |file: &Path, e: std::io::Error| Error::Io {
        path: file.to_path_buf(),
        message: e.to_string(),
    };
    let mut written = 0;
    for name in listed.lines().filter(|l| !l.is_empty()) {
        let below = name
            .strip_prefix(prefix)
            .unwrap_or(name)
            .trim_start_matches('/');
        let text = git(dir, "", "", &["show", &format!("{rev}:{name}")])?;
        let file = out.join(below);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::write(&file, format!("{text}\n")).map_err(|e| io(&file, e))?;
        written += 1;
    }
    Ok(written)
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

/// Whether `sha` deletes `path` from the fetched `base`, or adds or removes a line of it holding
/// one of `markers`, case-insensitively; true when git can't tell.
#[must_use]
pub fn secret_material(dir: &Path, base: &str, sha: &str, path: &str, markers: &[String]) -> bool {
    let range = format!("refs/remotes/origin/{base}...{sha}");
    let Ok(diff) = git(dir, "", "", &["diff", "--unified=0", &range, "--", path]) else {
        return true;
    };
    diff.contains("\ndeleted file mode")
        || diff.lines().any(|l| {
            let changed = (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"));
            let l = l.to_lowercase();
            changed && markers.iter().any(|m| l.contains(&m.to_lowercase()))
        })
}

/// Whether `path` on `sha` differs from the fetched `base` only in comments; see
/// `.claude/decisions/2026-11-17-security-auto-covers-routine-owner-asks.md` §4.
#[must_use]
pub fn comments_only(dir: &Path, base: &str, sha: &str, path: &str) -> bool {
    let show = |rev: &str| git(dir, "", "", &["show", &format!("{rev}:{path}")]).ok();
    let (Some(old), Some(new)) = (show(&format!("refs/remotes/origin/{base}")), show(sha)) else {
        return false;
    };
    uncommented(path, &old).is_some_and(|o| Some(o) == uncommented(path, &new))
}

/// Whether `path` on `sha` only renames the fetched `base`'s capabilities; see
/// `.claude/decisions/2026-11-17-security-auto-covers-routine-owner-asks.md`'s amendment.
#[must_use]
pub fn renames_only(dir: &Path, base: &str, sha: &str, path: &str) -> bool {
    let show = |rev: &str| git(dir, "", "", &["show", &format!("{rev}:{path}")]).ok();
    let (Some(old), Some(new)) = (show(&format!("refs/remotes/origin/{base}")), show(sha)) else {
        return false;
    };
    same_capabilities(&old, &new)
}

/// Whether two capability catalogs differ only in their capabilities' names: every other key
/// equal, and the capabilities' bodies equal as multisets; false when either doesn't parse.
#[must_use]
pub fn same_capabilities(old: &str, new: &str) -> bool {
    use serde_json::{Map, Value};
    let parse = |t: &str| serde_saphyr::from_str::<Value>(t).ok();
    let (Some(mut old), Some(mut new)) = (parse(old), parse(new)) else {
        return false;
    };
    let caps = |v: &mut Value| -> Option<Map<String, Value>> {
        v.as_object_mut()?
            .remove("capabilities")?
            .as_object()
            .cloned()
    };
    let (Some(a), Some(b)) = (caps(&mut old), caps(&mut new)) else {
        return false;
    };
    let sorted = |m: &Map<String, Value>| {
        let mut v: Vec<String> = m.values().map(ToString::to_string).collect();
        v.sort();
        v
    };
    let kept = a.iter().all(|(k, v)| b.get(k).is_none_or(|w| w == v));
    kept && sorted(&a) == sorted(&b) && old == new
}

/// `text` without its comments, as tokens: Rust through proc-macro2's lexer, doc attributes
/// dropped, and YAML without `#` comments; none for any other file, or text that doesn't lex.
#[must_use]
pub fn uncommented(path: &str, text: &str) -> Option<Vec<String>> {
    if Path::new(path).extension().is_some_and(|e| e == "rs") {
        let stream: proc_macro2::TokenStream = text.parse().ok()?;
        let mut out = Vec::new();
        rust_tokens(stream, &mut out);
        return Some(out);
    }
    if !Path::new(path)
        .extension()
        .is_some_and(|e| e == "yaml" || e == "yml")
    {
        return None;
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_end();
        if t.ends_with('|') || t.ends_with('>') || t.ends_with("|-") || t.ends_with(">-") {
            return None;
        }
        if t.trim_start().starts_with('#') {
            continue;
        }
        let (mut quote, mut cut) = (None, t.len());
        for (i, c) in t.char_indices() {
            match (quote, c) {
                (None, '\'' | '"') => quote = Some(c),
                (Some(q), _) if c == q => quote = None,
                (None, '#') if i == 0 || t[..i].ends_with(' ') => {
                    cut = i;
                    break;
                }
                _ => {}
            }
        }
        out.push(t[..cut].trim_end().to_string());
    }
    Some(out)
}

fn rust_tokens(stream: proc_macro2::TokenStream, out: &mut Vec<String>) {
    use proc_macro2::TokenTree;
    let mut it = stream.into_iter().peekable();
    while let Some(t) = it.next() {
        if let TokenTree::Punct(p) = &t
            && p.as_char() == '#'
        {
            let bang = matches!(it.peek(), Some(TokenTree::Punct(b)) if b.as_char() == '!');
            let mut ahead = it.clone();
            if bang {
                ahead.next();
            }
            if let Some(TokenTree::Group(g)) = ahead.peek()
                && g.delimiter() == proc_macro2::Delimiter::Bracket
                && g.stream()
                    .into_iter()
                    .next()
                    .is_some_and(|f| f.to_string() == "doc")
            {
                ahead.next();
                it = ahead;
                continue;
            }
        }
        match t {
            TokenTree::Group(g) => {
                out.push(format!("{:?}(", g.delimiter()));
                rust_tokens(g.stream(), out);
                out.push(")".into());
            }
            other => out.push(other.to_string()),
        }
    }
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

/// `(name, email)` of a git identity written `Name <email>`, when it is one.
#[must_use]
pub fn identity(s: &str) -> Option<(String, String)> {
    let (name, rest) = s.trim().split_once('<')?;
    let email = rest.strip_suffix('>')?;
    let name = name.trim();
    let bad = |p: &str| p.is_empty() || p.contains(['<', '>', '\n', '\r']);
    if bad(name) || bad(email) || !email.contains('@') {
        return None;
    }
    Some((name.to_string(), email.to_string()))
}

/// Commits `sha`'s tree onto `base`'s head as one commit with `message`, and pushes it to
/// `remote`'s `base`, then deletes `branch` there; the commit pushed. `base`'s head must be an
/// ancestor of `sha`, so the commit's tree is exactly the checked one; the push is never
/// forced, so a `base` that moved since the fetch is refused. With `author` (`Name <email>`),
/// the commit's author is it; its committer is always `pigeon`.
///
/// # Errors
///
/// [`Error::Git`] when `base` isn't an ancestor of `sha`, `author` isn't `Name <email>`, or
/// git fails.
#[allow(clippy::too_many_arguments)]
pub fn squash_onto(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    branch: &str,
    sha: &str,
    message: &str,
    author: Option<&str>,
) -> Result<String, Error> {
    let author = match author {
        Some(a) => Some(identity(a).ok_or_else(|| Error::Git {
            args: "commit-tree".into(),
            message: format!("the author `{a}` is not `Name <email>`"),
        })?),
        None => None,
    };
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
            message: format!("{base} {STALE_BASE} {branch}'s base; rebase onto {base} and push"),
        });
    }
    let parent = git(dir, "", "", &["rev-parse", &on])?;
    let tree = format!("{sha}^{{tree}}");
    let mut cmd = command("", "");
    if let Some((name, email)) = &author {
        cmd.env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email);
    }
    let commit = cmd
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
    push_retrying(dir, remote, token, &format!("{new}:refs/heads/{base}"))?;
    push_retrying(dir, remote, token, &format!(":refs/heads/{branch}"))?;
    Ok(new)
}

/// Pushes `refspec` to `remote`, again after each of [`push::BACKOFF`]'s waits while git's
/// refusal is transient; the last such refusal names [`TRANSIENT_PUSH`].
fn push_retrying(dir: &Path, remote: &str, token: &str, refspec: &str) -> Result<(), Error> {
    let mut waits = push::BACKOFF.iter();
    loop {
        match git(dir, remote, token, &["push", "-q", remote, refspec]) {
            Ok(_) => return Ok(()),
            Err(Error::Git { message, .. }) if push::transient(&message) => match waits.next() {
                Some(&s) => std::thread::sleep(Duration::from_secs(s)),
                None => {
                    return Err(Error::Git {
                        args: "push".into(),
                        message: format!(
                            "{refspec} {TRANSIENT_PUSH} {} times: {message}",
                            push::BACKOFF.len() + 1
                        ),
                    });
                }
            },
            Err(e) => return Err(e),
        }
    }
}

/// Deletes `refname` from `remote` from a new repository at `dir`; a ref already gone is no
/// error.
///
/// # Errors
///
/// [`Error::Git`] when git fails; [`Error::Io`] when `dir` can't be made.
pub fn delete_ref(dir: &Path, remote: &str, token: &str, refname: &str) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    git(dir, remote, token, &["init", "-q"])?;
    match git(
        dir,
        remote,
        token,
        &["push", "-q", remote, &format!(":{refname}")],
    ) {
        Err(Error::Git { message, .. }) if message.contains("remote ref does not exist") => Ok(()),
        other => other.map(|_| ()),
    }
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

/// Fetches `branch` and every tag from `remote` into a new repository at `dir`, and checks out
/// `sha` when it is still `branch`'s head; `false`, with nothing checked out, when it moved.
///
/// # Errors
///
/// [`Error::Git`] when git fails; [`Error::Io`] when `dir` can't be made.
pub fn checkout_head(
    dir: &Path,
    remote: &str,
    token: &str,
    branch: &str,
    sha: &str,
) -> Result<bool, Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    git(dir, remote, token, &["init", "-q"])?;
    let head = fetch_head(dir, remote, token, branch)?;
    if head != sha {
        return Ok(false);
    }
    fetch_tags(dir, remote, token)?;
    git(
        dir,
        remote,
        token,
        &["-c", "advice.detachedHead=false", "checkout", "-q", sha],
    )?;
    Ok(true)
}

/// `branch`'s head on `remote`, fetched into `dir` as `refs/remotes/origin/<branch>`.
fn fetch_head(dir: &Path, remote: &str, token: &str, branch: &str) -> Result<String, Error> {
    let r = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    git(
        dir,
        remote,
        token,
        &["fetch", "-q", "--no-tags", remote, &r],
    )?;
    git(
        dir,
        remote,
        token,
        &["rev-parse", &format!("refs/remotes/origin/{branch}")],
    )
}

/// How [`land`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landed {
    /// The commit, pushed to the branch, with its tags.
    Pushed(String),
    /// The branch moved past the checkout, so nothing was pushed.
    Moved,
}

/// Commits everything in the checkout at `dir` with `message` (its author `author`,
/// `Name <email>`, else `pigeon`; its committer `pigeon`), tags the commit with each of `tags`,
/// pushes it to `remote`'s `branch` as a fast-forward, then pushes the tags. A refused push
/// whose branch has moved is [`Landed::Moved`], and pushes no tag.
///
/// # Errors
///
/// [`Error::Git`] when `author` isn't `Name <email>`, or git fails.
pub fn land(
    dir: &Path,
    remote: &str,
    token: &str,
    branch: &str,
    message: &str,
    author: Option<&str>,
    tags: &[String],
) -> Result<Landed, Error> {
    let author = match author {
        Some(a) => Some(identity(a).ok_or_else(|| Error::Git {
            args: "commit".into(),
            message: format!("the author `{a}` is not `Name <email>`"),
        })?),
        None => None,
    };
    let from = git(dir, "", "", &["rev-parse", "HEAD"])?;
    git(dir, "", "", &["add", "-A"])?;
    let mut cmd = command("", "");
    if let Some((name, email)) = &author {
        cmd.env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email);
    }
    let out = cmd
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=pigeon",
            "-c",
            "user.email=pigeon@users.noreply.pigeon.dev",
            "commit",
            "-q",
            "--no-verify",
            "-m",
            message,
        ])
        .output()
        .map_err(|e| Error::Git {
            args: "commit".into(),
            message: e.to_string(),
        })?;
    if !out.status.success() {
        return Err(Error::Git {
            args: "commit".into(),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    let commit = git(dir, "", "", &["rev-parse", "HEAD"])?;
    for t in tags {
        git(dir, "", "", &["tag", t, &commit])?;
    }
    let to = format!("{commit}:refs/heads/{branch}");
    if let Err(e) = git(dir, remote, token, &["push", "-q", remote, &to]) {
        return match fetch_head(dir, remote, token, branch) {
            Ok(head) if head != from => Ok(Landed::Moved),
            _ => Err(e),
        };
    }
    if !tags.is_empty() {
        let refs: Vec<String> = tags
            .iter()
            .map(|t| format!("refs/tags/{t}:refs/tags/{t}"))
            .collect();
        let mut args = vec!["push", "-q", remote];
        args.extend(refs.iter().map(String::as_str));
        git(dir, remote, token, &args)?;
    }
    Ok(Landed::Pushed(commit))
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
    /// Whether the change's own diff is the same: by `git patch-id --stable`, or its added and
    /// removed lines, blank lines aside, with no binary file ([`own_lines`]).
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
    /// The head adds nothing to the base: its patch already landed, so nothing was pushed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub landed: bool,
    /// The paths the base changed since the head's fork; empty when current.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub main_files: Vec<String>,
    /// The paths the new head changes on the base, checked out in the work tree; empty when none
    /// was pushed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub head_files: Vec<String>,
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

/// The lines `from..to` adds and removes, by file, without context or blank lines: what an
/// approver approved, which a union merge's or a neighbour's lines don't change; none when a
/// binary file changed. See the concurrent-sessions ADR's 2026-11-13 amendment on carries.
///
/// # Errors
///
/// When git fails.
pub fn own_lines(dir: &Path, from: &str, to: &str) -> Result<Option<Vec<String>>, Error> {
    let diff = git(
        dir,
        "",
        "",
        &["diff", "-U0", "--no-color", "--no-ext-diff", from, to],
    )?;
    let mut out = Vec::new();
    for line in diff.lines() {
        if line.starts_with("Binary files ") || line == "GIT binary patch" {
            return Ok(None);
        }
        if line.starts_with("diff --git ") {
            out.push(line.to_string());
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        let own = line.starts_with('+') || line.starts_with('-');
        if own && !line[1..].trim().is_empty() {
            out.push(line.to_string());
        }
    }
    Ok(Some(out))
}

/// A digest of `sha`'s [`own_lines`] against its merge-base with `base`; none for a binary file,
/// or when git can't say. See the merge-path ADR §8.
#[must_use]
pub fn own_digest(dir: &Path, base: &str, sha: &str) -> Option<String> {
    use sha2::Digest as _;
    let fork = git(dir, "", "", &["merge-base", base, sha]).ok()?;
    let lines = own_lines(dir, fork.trim(), sha).ok().flatten()?;
    let digest = sha2::Sha256::digest(lines.join("\n").as_bytes());
    Some(crate::core::approval::grants::hex(&digest))
}

/// `sha`'s patch-id against its merge-base with `base`, or none when git can't say.
#[must_use]
pub fn head_patch(dir: &Path, base: &str, sha: &str) -> Option<String> {
    let fork = git(dir, "", "", &["merge-base", base, sha]).ok()?;
    patch_id(dir, fork.trim(), sha).ok()
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

/// Has git merge paths matching each `union` glob by union in `dir`.
fn merge_by_union(dir: &Path, union: &[&str]) -> Result<(), Error> {
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
        })
}

/// The open changes in `others` (number, branch) whose heads would conflict with `sha` once both
/// are restacked onto `base`, each with those paths: fetched read-only, each head rebased onto
/// `base` in memory and the two merged with `base` as their merge base, paths matching a `union`
/// glob by union as a restack would, nothing pushed; a branch that can't be fetched, or conflicts
/// with `base` itself, is skipped; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5 and its
/// 2026-11-16 amendment.
#[must_use]
pub fn forecast(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    sha: &str,
    others: &[(u32, String)],
    union: &[&str],
) -> Vec<(u32, Vec<String>)> {
    if merge_by_union(dir, union).is_err() {
        return Vec::new();
    }
    let main = format!("refs/remotes/origin/{base}");
    // `head`'s tree rebased onto `main`; none when it conflicts with `main` itself.
    let onto_main = |head: &str| -> Option<String> {
        let fork = git(dir, "", "", &["merge-base", &main, head]).ok()?;
        merge_from(dir, &fork, &main, head).ok()?.ok()
    };
    let Some(mine) = onto_main(sha) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (n, branch) in others {
        let local = format!("refs/forecast/{n}");
        let spec = format!("+refs/heads/{branch}:{local}");
        if git(dir, remote, token, &["fetch", "-q", remote, &spec]).is_err() {
            continue;
        }
        let Some(theirs) = onto_main(&local) else {
            continue;
        };
        if let Ok(Err((_, paths))) = merge_from(dir, &main, &mine, &theirs)
            && !paths.is_empty()
        {
            out.push((*n, paths));
        }
    }
    out
}

/// `ours` and `theirs` merged in `dir`'s work tree: the tree, or the tree with conflict markers and
/// the paths in conflict.
fn merge_tree(
    dir: &Path,
    ours: &str,
    theirs: &str,
) -> Result<Result<String, (String, Vec<String>)>, Error> {
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
        Some(1) => Ok(Err((tree, lines.map(str::to_string).collect()))),
        _ => Err(Error::Git {
            args: "merge-tree".into(),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
    }
}

/// `merge_tree` of `ours` and `theirs` with `base` as their merge base: each tree recommitted
/// on a root commit of `base`'s tree, as `merge-tree --merge-base` would on git 2.40.
fn merge_from(
    dir: &Path,
    base: &str,
    ours: &str,
    theirs: &str,
) -> Result<Result<String, (String, Vec<String>)>, Error> {
    let tree = |c: &str| git(dir, "", "", &["rev-parse", &format!("{c}^{{tree}}")]);
    let who = [
        "-c",
        "user.name=pigeon",
        "-c",
        "user.email=pigeon@users.noreply.pigeon.dev",
    ];
    let root = tree(base)?;
    let mut args = who.to_vec();
    args.extend(["commit-tree", root.as_str(), "-m", "base"]);
    let b = git(dir, "", "", &args)?;
    let o = commit_tree(dir, &tree(ours)?, &b, "ours")?;
    let t = commit_tree(dir, &tree(theirs)?, &b, "theirs")?;
    merge_tree(dir, &o, &t)
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

/// The longest the lockfile's regeneration runs.
const LOCK_TIMEOUT: Duration = Duration::from_mins(5);

/// The merged tree `marked`, its `Cargo.lock` the change's regenerated by
/// `cargo update --workspace --offline`, when that file alone conflicts and the result is
/// accepted ([`lockfile::accepts`]); none otherwise, with the work tree back on `onto`. See the
/// merge-path ADR §2.
fn regenerated_lock(
    dir: &Path,
    from: &str,
    onto: &str,
    sha: &str,
    marked: &str,
    conflicts: &[String],
) -> Result<Option<String>, Error> {
    if !lockfile::regenerates(conflicts) {
        return Ok(None);
    }
    let show = |rev: &str| {
        git(
            dir,
            "",
            "",
            &["show", &format!("{rev}:{}", lockfile::LOCKFILE)],
        )
    };
    let fork = show(from).unwrap_or_default();
    let main = show(onto)?;
    let change = show(sha)?;
    git(dir, "", "", &["read-tree", "-u", "--reset", marked])?;
    let path = dir.join(lockfile::LOCKFILE);
    std::fs::write(&path, format!("{change}\n")).map_err(|e| Error::Io {
        path: path.clone(),
        message: e.to_string(),
    })?;
    let update = ["cargo", "update", "--workspace", "--offline"].map(String::from);
    let (ok, _) = exec(dir, &update, LOCK_TIMEOUT, "");
    let regenerated = std::fs::read_to_string(&path).unwrap_or_default();
    if !(ok && lockfile::accepts(&fork, &main, &change, &regenerated)) {
        git(dir, "", "", &["read-tree", "-u", "--reset", onto])?;
        return Ok(None);
    }
    git(dir, "", "", &["add", lockfile::LOCKFILE])?;
    git(dir, "", "", &["write-tree"]).map(Some)
}

/// Replays `sha`, `branch`'s head checked out in `dir` by [`checkout`], onto `base` as it now
/// stands, as one commit carrying the message of the change's first commit, and pushes it to
/// `branch` only if `sha` is still its head. Paths matching a `union` glob merge by union; see
/// `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.
/// A conflict in `Cargo.lock` alone is regenerated; see the merge-path ADR §2.
///
/// # Errors
///
/// [`Error::Git`] when git fails, or the push loses to a newer head.
#[expect(clippy::too_many_arguments, reason = "the job's spec, spelled out")]
pub fn restack(
    dir: &Path,
    remote: &str,
    token: &str,
    base: &str,
    branch: &str,
    sha: &str,
    since: Option<&str>,
    union: &[&str],
) -> Result<Restack, Error> {
    merge_by_union(dir, union)?;
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
        landed: false,
        main_files: Vec::new(),
        head_files: Vec::new(),
    };
    if fork == onto {
        return Ok(r);
    }
    let mine = changed_since(dir, &fork, sha)?;
    let theirs = changed_since(dir, &fork, &onto)?;
    r.main_files.clone_from(&theirs);
    let detached = ["-c", "advice.detachedHead=false", "checkout", "-q", &onto];
    git(dir, "", "", &detached)?;
    // Built on another change: its own commits start past that change's last head, which is
    // also the merge's base, so lines that change added and this one edits merge as edits; see
    // the yield ADR's 2026-11-14 amendment on restacks from the change built on.
    let from = since
        .filter(|s| git(dir, "", "", &["merge-base", "--is-ancestor", s, sha]).is_ok())
        .unwrap_or(&fork);
    let merged = if from == fork {
        merge_tree(dir, &onto, sha)?
    } else {
        merge_from(dir, from, &onto, sha)?
    };
    let tree = match merged {
        Ok(tree) => tree,
        Err((marked, conflicts)) => {
            if let Some(tree) = regenerated_lock(dir, from, &onto, sha, &marked, &conflicts)? {
                tree
            } else {
                (r.context, r.messages) = conflict_context(dir, &fork, &onto, sha, &conflicts)?;
                r.relation = Relation::Conflicting;
                r.same_patch = false;
                r.conflicts = conflicts;
                return Ok(r);
            }
        }
    };
    // A merge that leaves the base's tree as it was adds nothing: the patch already landed.
    if tree == git(dir, "", "", &["rev-parse", &format!("{onto}^{{tree}}")])? {
        r.relation = Relation::Clean;
        r.same_patch = false;
        r.landed = true;
        return Ok(r);
    }
    let commits = git(
        dir,
        "",
        "",
        &["rev-list", "--reverse", &format!("{from}..{sha}")],
    )?;
    let first = commits.lines().next().unwrap_or(sha);
    let message = git(dir, "", "", &["log", "-1", "--format=%B", first])?;
    let new = commit_tree(dir, &tree, &onto, &format!("{message}\n"))?;
    r.relation = if mine.iter().any(|p| theirs.contains(p)) {
        Relation::Clean
    } else {
        Relation::Disjoint
    };
    // Counted from the change built on, when there is one; see the always-listed ADR's
    // 2026-11-15 amendment on changes built on another.
    let mine = own_lines(dir, from, sha)?;
    r.same_patch = patch_id(dir, from, sha)? == patch_id(dir, &onto, &new)?
        || (mine.is_some() && mine == own_lines(dir, &onto, &new)?);
    r.head_files = changed_since(dir, &onto, &new)?;
    git(
        dir,
        "",
        "",
        &["-c", "advice.detachedHead=false", "checkout", "-q", &new],
    )?;
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
