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

/// Ensures the namespace and each repository exist, importing a new one from its mirror;
/// idempotent.
///
/// # Errors
///
/// [`Error::Tool`] naming the API call that failed.
pub fn apply(client: &Client<'_>, instance: &Instance) -> Result<Vec<Applied>, Error> {
    let ns = &instance.namespace;
    client.namespace(ns).map_err(api_error)?;
    let mut out = Vec::new();
    for d in &instance.repos {
        let (outcome, repo) = match client.repo(ns, &d.name).map_err(api_error)? {
            Some(r) => (Outcome::Unchanged, r),
            None => match &d.mirror {
                Some(url) => (
                    Outcome::Imported,
                    client
                        .import(ns, &d.name, url, &d.branch)
                        .map_err(api_error)?,
                ),
                None => (
                    Outcome::Created,
                    client
                        .create(ns, &d.name, d.description.as_deref())
                        .map_err(api_error)?,
                ),
            },
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

/// Mirrors repository `name`'s default branch and tags, and nothing else, to `to`, forcing it
/// to follow Artifacts; staged in `<root>/.cobot/git/<name>.git`. The push uses the user's own
/// git credentials, never the Artifacts token.
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
        &stage(root, name)?,
    )
}

fn stage(root: &Path, name: &str) -> Result<PathBuf, Error> {
    let dir = root.join(MIRROR_DIR).join(format!("{name}.git"));
    if !dir.join("HEAD").is_file() {
        fs::create_dir_all(&dir).map_err(|e| Error::Io {
            path: dir.clone(),
            message: e.to_string(),
        })?;
        checked(
            Command::new("git")
                .args(["init", "--quiet", "--bare"])
                .arg(&dir)
                .output(),
            "git init",
        )?;
    }
    Ok(dir)
}

/// [`mirror`] from `remote` with `token`, through the bare repository `dir`.
///
/// # Errors
///
/// [`Error::Tool`] when git fails.
pub fn mirror_from(
    remote: &str,
    token: &str,
    branch: &str,
    name: &str,
    to: &str,
    dir: &Path,
) -> Result<Mirrored, Error> {
    let heads = format!("+refs/heads/{branch}:refs/heads/{branch}");
    checked(
        command(remote, token)
            .arg("--git-dir")
            .arg(dir)
            .args(["fetch", "--quiet", "--prune", "--force", remote, &heads])
            .arg("+refs/tags/*:refs/tags/*")
            .output(),
        "git fetch",
    )?;
    let mut push = Command::new("git");
    push.env("GIT_TERMINAL_PROMPT", "0")
        .arg("--git-dir")
        .arg(dir)
        .args(["push", "--quiet", "--atomic", to, &heads])
        .arg("+refs/tags/*:refs/tags/*");
    checked(push.output(), "git push")?;
    let listed = checked(
        Command::new("git")
            .arg("--git-dir")
            .arg(dir)
            .args(["for-each-ref", "--format=%(refname) %(objectname)"])
            .arg(format!("refs/heads/{branch}"))
            .arg("refs/tags")
            .output(),
        "git for-each-ref",
    )?;
    let mut refs: Vec<(String, String)> = listed
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(r, s)| (r.to_string(), s.to_string()))
        .collect();
    refs.sort();
    Ok(Mirrored {
        name: name.to_string(),
        to: to.to_string(),
        refs,
    })
}
