//! Read-only git queries for `release`.

use std::path::Path;
use std::process::Command;

use crate::Error;

/// One first-parent commit.
#[derive(Debug, Clone)]
pub struct Commit {
    /// Full SHA.
    pub sha: String,
    /// UTC commit date, `YYYY-MM-DD`.
    pub date: String,
    /// The subject line.
    pub subject: String,
    /// Paths the commit changed.
    pub paths: Vec<String>,
}

/// Runs git in `root` and returns stdout.
pub fn run(root: &Path, args: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("TZ", "UTC")
        .output()
        .map_err(|e| Error::Git {
            args: args.join(" "),
            message: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(Error::Git {
            args: args.join(" "),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether `rev` names a commit.
pub fn is_commit(root: &Path, rev: &str) -> bool {
    !rev.bytes().all(|b| b == b'0')
        && run(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ],
        )
        .is_ok()
}

/// First-parent commits after `from` (or all of history) up to `to`, oldest first.
pub fn commits(root: &Path, from: Option<&str>, to: &str) -> Result<Vec<Commit>, Error> {
    let range = match from {
        Some(f) if is_commit(root, f) => format!("{f}..{to}"),
        _ => to.to_string(),
    };
    let log = run(
        root,
        &[
            "log",
            "--first-parent",
            "--reverse",
            "--date=format-local:%Y-%m-%d",
            "--format=%H%x09%cd%x09%s",
            &range,
        ],
    )?;
    let mut commits = Vec::new();
    for line in log.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(sha), Some(date), Some(subject)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let paths = run(
            root,
            &[
                "diff-tree",
                "-z",
                "--no-commit-id",
                "--name-only",
                "-r",
                "--root",
                "-m",
                "--first-parent",
                sha,
            ],
        )?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
        commits.push(Commit {
            sha: sha.into(),
            date: date.into(),
            subject: subject.into(),
            paths,
        });
    }
    Ok(commits)
}

/// The newest first-parent commit up to `to` whose subject starts with `prefix`.
pub fn last_with_subject(root: &Path, to: &str, prefix: &str) -> Result<Option<String>, Error> {
    let log = run(root, &["log", "--first-parent", "--format=%H%x09%s", to])?;
    Ok(log.lines().find_map(|l| {
        let (sha, subject) = l.split_once('\t')?;
        subject.starts_with(prefix).then(|| sha.to_string())
    }))
}

/// The newest `<unit>/v*` tag, by version.
pub fn last_tag(root: &Path, unit: &str) -> Result<Option<String>, Error> {
    let tags = run(
        root,
        &["tag", "--list", &format!("{unit}/v*"), "--sort=-v:refname"],
    )?;
    Ok(tags.lines().next().map(str::to_string))
}

/// The UTC date of `rev`.
pub fn date(root: &Path, rev: &str) -> Result<String, Error> {
    Ok(run(
        root,
        &[
            "log",
            "-1",
            "--date=format-local:%Y-%m-%d",
            "--format=%cd",
            rev,
        ],
    )?
    .trim()
    .to_string())
}
