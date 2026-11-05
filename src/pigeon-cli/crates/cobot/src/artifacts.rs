//! Cloudflare Artifacts, git repositories on Cloudflare, through the REST API with a run's own
//! token; see `.claude/decisions/2026-11-05-development-git-hosting-cloudflare-artifacts.md`.

use serde_json::{Value, json};

use crate::cloudflare::Api;

/// The `x-cobot.api` of a kind this module applies.
pub const API: &str = "artifacts";

/// How long a minted git token lives, in seconds.
pub const TOKEN_TTL: u64 = 900;

/// A repository, as the API describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Its name.
    pub name: String,
    /// Its git remote URL.
    pub remote: String,
    /// Its default branch.
    pub default_branch: Option<String>,
}

/// What a git token may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Fetch and clone.
    Read,
    /// Push too.
    Write,
}

impl Scope {
    fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Artifacts in one account.
#[derive(Debug)]
pub struct Client<'a> {
    api: &'a Api,
    account: String,
}

fn repo_of(v: &Value, name: &str) -> Result<Repo, String> {
    let remote = ["remote", "url", "git_url"]
        .iter()
        .find_map(|k| v[*k].as_str())
        .ok_or_else(|| format!("repository `{name}`: the API gave no remote"))?;
    Ok(Repo {
        name: v["name"].as_str().unwrap_or(name).to_string(),
        remote: remote.to_string(),
        default_branch: v["default_branch"].as_str().map(str::to_string),
    })
}

impl<'a> Client<'a> {
    /// Artifacts in the one account `api`'s token sees.
    ///
    /// # Errors
    ///
    /// A message when the token sees no account or more than one.
    pub fn new(api: &'a Api) -> Result<Self, String> {
        Ok(Self {
            account: api.account()?,
            api,
        })
    }

    /// The account's ID, for messages, as its last four characters only.
    #[must_use]
    pub fn account_hint(&self) -> String {
        let n = self.account.len();
        format!("…{}", &self.account[n.saturating_sub(4)..])
    }

    fn namespaces(&self) -> String {
        format!("/accounts/{}/artifacts/namespaces", self.account)
    }

    fn repos(&self, ns: &str) -> String {
        format!("{}/{ns}/repos", self.namespaces())
    }

    /// Ensures namespace `ns` exists; whether it was created.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn namespace(&self, ns: &str) -> Result<bool, String> {
        if self
            .api
            .find(&format!("{}/{ns}", self.namespaces()))?
            .is_some()
        {
            return Ok(false);
        }
        self.api
            .post(&self.namespaces(), &json!({ "name": ns }))
            .map(|_| true)
    }

    /// Repository `name` in `ns`, if it exists.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn repo(&self, ns: &str, name: &str) -> Result<Option<Repo>, String> {
        self.api
            .find(&format!("{}/{name}", self.repos(ns)))?
            .map(|v| repo_of(&v, name))
            .transpose()
    }

    /// Creates repository `name` in `ns`, empty.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn create(&self, ns: &str, name: &str, description: Option<&str>) -> Result<Repo, String> {
        let mut body = json!({ "name": name });
        if let Some(d) = description {
            body["description"] = json!(d);
        }
        let v = self.api.post(&self.repos(ns), &body)?;
        repo_of(&v, name)
    }

    /// Imports `url`, a public HTTPS remote, into repository `name` in `ns`.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn import(&self, ns: &str, name: &str, url: &str, branch: &str) -> Result<Repo, String> {
        let v = self.api.post(
            &format!("{}/{name}/import", self.repos(ns)),
            &json!({ "url": url, "branch": branch }),
        )?;
        self.repo(ns, name)?.map_or_else(|| repo_of(&v, name), Ok)
    }

    /// A token for repository `name` in `ns`, valid for [`TOKEN_TTL`] seconds.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed; never the token.
    pub fn token(&self, ns: &str, name: &str, scope: Scope) -> Result<String, String> {
        let v = self.api.post(
            &format!("{}/{name}/tokens", self.repos(ns)),
            &json!({ "scope": scope.word(), "ttl": TOKEN_TTL }),
        )?;
        v.as_str()
            .or_else(|| v["token"].as_str())
            .or_else(|| v["value"].as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("repository `{name}`: the API gave no token"))
    }
}
