//! An instance's git repositories on Cloudflare Artifacts, and their GitHub mirrors; see
//! `.claude/decisions/2026-11-05-development-git-hosting-cloudflare-artifacts.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use crate::Error;
use crate::artifacts::{self, Client, Repo, Scope};

/// The kind of a git repository unit.
pub const KIND: &str = "cloudflare/git-repo";

/// Where mirrors are staged, under the instance's root.
pub const MIRROR_DIR: &str = ".cobot/git";

/// One declared repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    /// Its unit ID, `repos/<name>`.
    pub id: String,
    /// Its name: the ID's last segment.
    pub name: String,
    /// Its GitHub mirror, if any.
    pub mirror: Option<String>,
    /// Its default branch.
    pub branch: String,
    /// What it is.
    pub description: Option<String>,
}

/// An instance's namespace and repositories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// The namespace: the instance's name.
    pub namespace: String,
    /// Its repositories, by ID.
    pub repos: Vec<Declared>,
}

/// The repositories the manifest at `root` declares, its kinds read from `platform`.
///
/// # Errors
///
/// [`Error::Invalid`] when there is no manifest, or it has problems.
pub fn declared(root: &Path, platform: &Path) -> Result<Instance, Error> {
    let invalid = |path: String, message: String| {
        Error::Invalid(vec![crate::unit::Problem { path, message }])
    };
    let Some((m, problems)) = crate::manifest::load(root, platform) else {
        return Err(invalid(
            crate::manifest::FILE.into(),
            format!(
                "no instance manifest at {}; repositories are `{KIND}` units in it",
                root.join(crate::manifest::FILE).display()
            ),
        ));
    };
    if !problems.is_empty() {
        return Err(Error::Invalid(
            problems
                .into_iter()
                .map(|(path, message)| crate::unit::Problem { path, message })
                .collect(),
        ));
    }
    let repos = m
        .api_units(artifacts::API)
        .into_iter()
        .filter(|(_, e)| e.kind == KIND)
        .map(|(id, e)| Declared {
            id: id.to_string(),
            name: crate::manifest::name(id).to_string(),
            mirror: e.settings["mirror"].as_str().map(str::to_string),
            branch: e.settings["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
            description: e.settings["description"].as_str().map(str::to_string),
        })
        .collect();
    Ok(Instance {
        namespace: m.name,
        repos,
    })
}

/// What `apply` did to one repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It existed.
    Unchanged,
    /// It was created empty.
    Created,
    /// It was created from its mirror.
    Imported,
}

/// One repository after `apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Its name.
    pub name: String,
    /// What happened.
    pub outcome: Outcome,
    /// Its git remote.
    pub remote: String,
}

fn api_error(message: String) -> Error {
    Error::Tool {
        command: "artifacts".into(),
        message,
    }
}

/// Ensures the namespace and each repository exist: one with a mirror is imported, which creates
/// it, and an existing one with no ref at all is replaced by an import; idempotent.
///
/// # Errors
///
/// [`Error::Tool`] naming the API call that failed.
pub fn apply(client: &Client<'_>, instance: &Instance) -> Result<Vec<Applied>, Error> {
    let ns = &instance.namespace;
    client.namespace(ns).map_err(api_error)?;
    let mut out = Vec::new();
    for d in &instance.repos {
        let existing = client.repo(ns, &d.name).map_err(api_error)?;
        let (outcome, repo) = match (existing, &d.mirror) {
            (Some(r), Some(url)) => {
                let token = client.token(ns, &d.name, Scope::Read).map_err(api_error)?;
                if no_refs(&r.remote, &token)? {
                    client.delete(ns, &d.name).map_err(api_error)?;
                    match client.import(ns, &d.name, url, &d.branch) {
                        Ok(imported) => (Outcome::Imported, imported),
                        Err(e) => {
                            let restored = client
                                .create(ns, &d.name, &d.branch, d.description.as_deref())
                                .is_ok();
                            return Err(api_error(if restored {
                                format!("{e}; the empty repository is restored")
                            } else {
                                format!("{e}; the empty repository could not be restored")
                            }));
                        }
                    }
                } else {
                    (Outcome::Unchanged, r)
                }
            }
            (Some(r), None) => (Outcome::Unchanged, r),
            (None, Some(url)) => (
                Outcome::Imported,
                client
                    .import(ns, &d.name, url, &d.branch)
                    .map_err(api_error)?,
            ),
            (None, None) => (
                Outcome::Created,
                client
                    .create(ns, &d.name, &d.branch, d.description.as_deref())
                    .map_err(api_error)?,
            ),
        };
        out.push(Applied {
            name: d.name.clone(),
            outcome,
            remote: repo.remote,
        });
    }
    Ok(out)
}

/// A declared repository's live record.
///
/// # Errors
///
/// [`Error::Tool`] when it does not exist yet, or the call fails.
pub fn live(
    client: &Client<'_>,
    instance: &Instance,
    name: &str,
) -> Result<(Declared, Repo), Error> {
    let d = instance
        .repos
        .iter()
        .find(|d| d.name == name)
        .cloned()
        .ok_or_else(|| Error::UnknownUnit(format!("repos/{name}")))?;
    let repo = client
        .repo(&instance.namespace, name)
        .map_err(api_error)?
        .ok_or_else(|| {
            api_error(format!(
                "repository `{name}` does not exist; run `pigeon cobot git apply`"
            ))
        })?;
    Ok((d, repo))
}

/// `git` with `token` sent as a bearer header to `remote` only, through the environment: never
/// in argv or `.git/config`.
#[must_use]
pub fn command(remote: &str, token: &str) -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", format!("http.{remote}.extraHeader"))
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        );
    c
}

/// Runs the user's `git args…` in `dir` against repository `name`, with a token of `scope`;
/// `{remote}` in an argument is replaced by the repository's remote.
///
/// # Errors
///
/// [`Error::Tool`] when the token cannot be minted or git cannot start.
pub fn run(
    client: &Client<'_>,
    instance: &Instance,
    name: &str,
    scope: Scope,
    args: &[String],
    dir: &Path,
) -> Result<ExitStatus, Error> {
    let (_, repo) = live(client, instance, name)?;
    let token = client
        .token(&instance.namespace, name, scope)
        .map_err(api_error)?;
    let args: Vec<String> = args
        .iter()
        .map(|a| a.replace("{remote}", &repo.remote))
        .collect();
    command(&repo.remote, &token)
        .args(&args)
        .current_dir(dir)
        .status()
        .map_err(|e| Error::Tool {
            command: "git".into(),
            message: e.to_string(),
        })
}

fn checked(output: std::io::Result<Output>, what: &str) -> Result<String, Error> {
    let fail = |message: String| Error::Tool {
        command: what.into(),
        message,
    };
    let o = output.map_err(|e| fail(e.to_string()))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(fail(String::from_utf8_lossy(&o.stderr).trim().to_string()))
    }
}

/// Whether `remote` has no ref at all yet, neither branch nor tag.
fn no_refs(remote: &str, token: &str) -> Result<bool, Error> {
    let out = checked(
        command(remote, token)
            .args(["ls-remote", "--refs", remote])
            .output(),
        "git ls-remote",
    )?;
    Ok(out.trim().is_empty())
}

/// The SHA of `refs/heads/<branch>` at `remote`, if it has one.
fn head(cmd: &mut Command, remote: &str, branch: &str) -> Result<Option<String>, Error> {
    let out = checked(
        cmd.args(["ls-remote", remote, &format!("refs/heads/{branch}")])
            .output(),
        "git ls-remote",
    )?;
    Ok(out.split_whitespace().next().map(str::to_string))
}

/// One repository's heads on Artifacts and on its mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Its name.
    pub name: String,
    /// Its remote.
    pub remote: String,
    /// The default branch's SHA on Artifacts.
    pub head: Option<String>,
    /// The default branch's SHA on the mirror, if it has one.
    pub mirror: Option<Option<String>>,
}

impl Status {
    /// Whether the mirror, if any, holds Artifacts' head.
    #[must_use]
    pub fn in_sync(&self) -> bool {
        self.mirror.as_ref().is_none_or(|m| *m == self.head)
    }
}

/// Each repository's heads, read with a read token.
///
/// # Errors
///
/// [`Error::Tool`] when a call or `git ls-remote` fails.
pub fn status(client: &Client<'_>, instance: &Instance) -> Result<Vec<Status>, Error> {
    let mut out = Vec::new();
    for d in &instance.repos {
        let (_, repo) = live(client, instance, &d.name)?;
        let token = client
            .token(&instance.namespace, &d.name, Scope::Read)
            .map_err(api_error)?;
        let head = head(&mut command(&repo.remote, &token), &repo.remote, &d.branch)?;
        let mirror = d
            .mirror
            .as_ref()
            .map(|m| head_plain(m, &d.branch))
            .transpose()?;
        out.push(Status {
            name: d.name.clone(),
            remote: repo.remote,
            head,
            mirror,
        });
    }
    Ok(out)
}

fn head_plain(remote: &str, branch: &str) -> Result<Option<String>, Error> {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0");
    head(&mut c, remote, branch)
}

/// What a mirror pushed: each ref and its SHA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirrored {
    /// The repository.
    pub name: String,
    /// The mirror.
    pub to: String,
    /// `(ref, sha)`, sorted.
    pub refs: Vec<(String, String)>,
}

/// Mirrors repository `name`'s default branch and tags, and nothing else, to `to`: the branch
/// only fast-forwards and tags are only added, while GitHub's release bot still writes there.
/// Staged in `<root>/.cobot/git/<name>.git`; the push uses the user's own git credentials,
/// never the Artifacts token.
///
/// # Errors
///
/// [`Error::Tool`] when a call or git fails.
pub fn mirror(
    client: &Client<'_>,
    instance: &Instance,
    name: &str,
    to: &str,
    root: &Path,
) -> Result<Mirrored, Error> {
    let (d, repo) = live(client, instance, name)?;
    let token = client
        .token(&instance.namespace, name, Scope::Read)
        .map_err(api_error)?;
    mirror_from(
        &repo.remote,
        &token,
        &d.branch,
        name,
        to,
        None,
        &stage(root, name)?,
    )
}

fn stage(root: &Path, name: &str) -> Result<PathBuf, Error> {
    let dir = root.join(MIRROR_DIR).join(format!("{name}.git"));
    bare(&dir)?;
    Ok(dir)
}

/// Makes `dir` a bare repository, unless it is one already.
///
/// # Errors
///
/// [`Error::Io`] or [`Error::Tool`] when it can't be made.
pub fn bare(dir: &Path) -> Result<(), Error> {
    if !dir.join("HEAD").is_file() {
        fs::create_dir_all(dir).map_err(|e| Error::Io {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;
        checked(
            Command::new("git")
                .args(["init", "--quiet", "--bare"])
                .arg(dir)
                .output(),
            "git init",
        )?;
    }
    Ok(())
}

/// [`mirror`] from `remote` with `token`, through the bare repository `dir`: the mirror's
/// branch only fast-forwards and its tags are only added; a mirror that moved ahead is refused.
/// With `to_token`, the push to `to` sends it as GitHub's `x-access-token` basic credential,
/// through the environment only; without it, the user's own git credentials apply.
///
/// # Errors
///
/// [`Error::Tool`] when git fails, or the mirror is ahead of or diverged from Artifacts.
pub fn mirror_from(
    remote: &str,
    token: &str,
    branch: &str,
    name: &str,
    to: &str,
    to_token: Option<&str>,
    dir: &Path,
) -> Result<Mirrored, Error> {
    let heads = format!("refs/heads/{branch}:refs/heads/{branch}");
    checked(
        command(remote, token)
            .arg("--git-dir")
            .arg(dir)
            .args(["fetch", "--quiet", "--force", remote, &format!("+{heads}")])
            .arg("refs/tags/*:refs/tags/*")
            .output(),
        "git fetch",
    )?;
    let mut push = match to_token {
        Some(t) => basic(dir, to, t),
        None => plain(dir),
    };
    push.args(["push", "--quiet", "--atomic", to, &heads])
        .arg("refs/tags/*:refs/tags/*");
    checked(push.output(), "git push").map_err(|e| match e {
        Error::Tool { message, .. } if message.contains("rejected") => Error::Tool {
            command: "git mirror".into(),
            message: format!(
                "{to} has commits or tags Artifacts lacks; run `pigeon cobot git sync {name}` first"
            ),
        },
        other => other,
    })?;
    Ok(Mirrored {
        name: name.to_string(),
        to: to.to_string(),
        refs: refs(dir, branch)?,
    })
}

/// Refuses a branch that is not a change's, so `main` and tags reach a mirror only through
/// [`mirror_from`].
fn change_branch(branch: &str) -> Result<String, Error> {
    match branch.strip_prefix("change/") {
        Some(rest) if !rest.is_empty() && !branch.contains("..") && !branch.contains(':') => {
            Ok(format!("refs/heads/{branch}"))
        }
        _ => Err(Error::Tool {
            command: "git mirror".into(),
            message: format!("`{branch}` is not a change's branch"),
        }),
    }
}

/// Force-pushes commit `sha` of `remote`'s `branch`, read with `token`, to `to` as that
/// branch, which must be `change/*`, through the bare repository `dir`; with `to_token` sent
/// as in [`mirror_from`]. See
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §2.
///
/// # Errors
///
/// [`Error::Tool`] when the branch is not a change's, the branch no longer holds `sha`, or git
/// fails.
pub fn push_change(
    remote: &str,
    token: &str,
    branch: &str,
    sha: &str,
    to: &str,
    to_token: Option<&str>,
    dir: &Path,
) -> Result<(), Error> {
    let r = change_branch(branch)?;
    checked(
        command(remote, token)
            .arg("--git-dir")
            .arg(dir)
            .args(["fetch", "--quiet", "--force", remote, &format!("+{r}:{r}")])
            .output(),
        "git fetch",
    )?;
    let has = plain(dir)
        .args(["merge-base", "--is-ancestor", sha, &r])
        .status()
        .is_ok_and(|s| s.success());
    if !has {
        return Err(Error::Tool {
            command: "git mirror".into(),
            message: format!("{branch} no longer holds {sha}"),
        });
    }
    let mut push = match to_token {
        Some(t) => basic(dir, to, t),
        None => plain(dir),
    };
    push.args(["push", "--quiet", "--force", to, &format!("{sha}:{r}")]);
    checked(push.output(), "git push").map(|_| ())
}

/// Deletes `branch`, which must be `change/*`, from `to`; one already gone is not an error.
///
/// # Errors
///
/// [`Error::Tool`] when the branch is not a change's, or git fails.
pub fn delete_change(
    branch: &str,
    to: &str,
    to_token: Option<&str>,
    dir: &Path,
) -> Result<(), Error> {
    let r = change_branch(branch)?;
    let mut push = match to_token {
        Some(t) => basic(dir, to, t),
        None => plain(dir),
    };
    push.args(["push", "--quiet", to, &format!(":{r}")]);
    match checked(push.output(), "git push") {
        Err(Error::Tool { message, .. }) if message.contains("remote ref does not exist") => Ok(()),
        other => other.map(|_| ()),
    }
}

/// `git --git-dir dir` with `token` sent to `to` only as the basic credential
/// `x-access-token:<token>`, as GitHub takes an App's installation token; through the
/// environment, never argv or a file.
fn basic(dir: &Path, to: &str, token: &str) -> Command {
    let auth = crate::imagebuild::base64(format!("x-access-token:{token}").as_bytes());
    let mut c = plain(dir);
    c.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", format!("http.{to}.extraHeader"))
        .env("GIT_CONFIG_VALUE_0", format!("Authorization: Basic {auth}"));
    c
}

/// `git --git-dir dir`, never prompting.
fn plain(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0").arg("--git-dir").arg(dir);
    c
}

/// The default branch and tags in `dir`, sorted.
fn refs(dir: &Path, branch: &str) -> Result<Vec<(String, String)>, Error> {
    let listed = checked(
        plain(dir)
            .args(["for-each-ref", "--format=%(refname) %(objectname)"])
            .arg(format!("refs/heads/{branch}"))
            .arg("refs/tags")
            .output(),
        "git for-each-ref",
    )?;
    let mut out: Vec<(String, String)> = listed
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(r, s)| (r.to_string(), s.to_string()))
        .collect();
    out.sort();
    Ok(out)
}

/// What a sync brought back from the mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    /// The repository.
    pub name: String,
    /// The default branch's move on Artifacts, `(from, to)`, when the mirror was ahead.
    pub advanced: Option<(String, String)>,
    /// Tags added to Artifacts.
    pub tags: usize,
}

/// Brings repository `name` up to its mirror `to`, where GitHub's release bot still commits and
/// tags: fast-forwards the default branch when Artifacts is behind, and adds the tags it lacks.
/// Staged in `<root>/.cobot/git/<name>.git`.
///
/// # Errors
///
/// [`Error::Tool`] when a call or git fails, or the two have diverged.
pub fn sync(
    client: &Client<'_>,
    instance: &Instance,
    name: &str,
    to: &str,
    root: &Path,
) -> Result<Synced, Error> {
    let (d, repo) = live(client, instance, name)?;
    let token = client
        .token(&instance.namespace, name, Scope::Write)
        .map_err(api_error)?;
    sync_from(
        &repo.remote,
        &token,
        &d.branch,
        name,
        to,
        &stage(root, name)?,
    )
}

/// [`sync`] from mirror `to` into `remote` with write `token`, through the bare repository `dir`.
///
/// # Errors
///
/// [`Error::Tool`] when git fails, or the two branches have diverged.
pub fn sync_from(
    remote: &str,
    token: &str,
    branch: &str,
    name: &str,
    to: &str,
    dir: &Path,
) -> Result<Synced, Error> {
    let ours = format!("refs/heads/{branch}");
    let theirs = format!("refs/mirror/{branch}");
    checked(
        command(remote, token)
            .arg("--git-dir")
            .arg(dir)
            .args([
                "fetch",
                "--quiet",
                "--force",
                remote,
                &format!("+{ours}:{ours}"),
            ])
            .output(),
        "git fetch",
    )?;
    let before: std::collections::BTreeSet<String> = checked(
        command(remote, token)
            .args(["ls-remote", "--tags", "--refs", remote])
            .output(),
        "git ls-remote",
    )?
    .lines()
    .filter_map(|l| l.split_whitespace().last())
    .map(str::to_string)
    .collect();
    checked(
        plain(dir)
            .args(["fetch", "--quiet", to, &format!("+{ours}:{theirs}")])
            .arg("refs/tags/*:refs/tags/*")
            .output(),
        "git fetch",
    )?;
    let sha = |r: &str| {
        checked(plain(dir).args(["rev-parse", r]).output(), "git rev-parse")
            .map(|s| s.trim().to_string())
    };
    let is_ancestor = |a: &str, b: &str| {
        plain(dir)
            .args(["merge-base", "--is-ancestor", a, b])
            .status()
            .is_ok_and(|s| s.success())
    };
    let (here, there) = (sha(&ours)?, sha(&theirs)?);
    let advanced = if here == there || is_ancestor(&there, &here) {
        None
    } else if is_ancestor(&here, &there) {
        Some((here.clone(), there.clone()))
    } else {
        return Err(Error::Tool {
            command: "git sync".into(),
            message: format!(
                "{name} has diverged: Artifacts {branch} is {here}, {to} is {there}; merge them by hand"
            ),
        });
    };
    let added = refs(dir, branch)?
        .into_iter()
        .filter(|(r, _)| r.starts_with("refs/tags/") && !before.contains(r))
        .count();
    let mut push = command(remote, token);
    push.arg("--git-dir")
        .arg(dir)
        .args(["push", "--quiet", "--atomic", remote])
        .arg("refs/tags/*:refs/tags/*");
    if advanced.is_some() {
        push.arg(format!("{theirs}:{ours}"));
    }
    checked(push.output(), "git push")?;
    Ok(Synced {
        name: name.to_string(),
        advanced,
        tags: added,
    })
}
