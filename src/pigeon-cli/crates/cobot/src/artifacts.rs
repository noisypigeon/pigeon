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
            .post(&self.namespaces(), &json!({ "namespace": ns }))
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
    pub fn create(
        &self,
        ns: &str,
        name: &str,
        branch: &str,
        description: Option<&str>,
    ) -> Result<Repo, String> {
        let mut body = json!({ "name": name, "default_branch": branch });
        if let Some(d) = description {
            body["description"] = json!(d);
        }
        let v = self.api.post(&self.repos(ns), &body)?;
        repo_of(&v, name)
    }

    /// Deletes repository `name` in `ns`.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn delete(&self, ns: &str, name: &str) -> Result<(), String> {
        self.api
            .delete(&format!("{}/{name}", self.repos(ns)))
            .map(|_| ())
    }

    /// Creates repository `name` in `ns` from `url`, a public HTTPS remote; the repository must
    /// not exist yet.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed.
    pub fn import(&self, ns: &str, name: &str, url: &str, branch: &str) -> Result<Repo, String> {
        let v = self.api.post_first(
            &format!("{}/{name}/import", self.repos(ns)),
            &[
                json!({ "source": { "url": url, "branch": branch } }),
                json!({ "source": { "url": url } }),
                json!({ "url": url }),
            ],
        )?;
        self.repo(ns, name)?.map_or_else(|| repo_of(&v, name), Ok)
    }

    /// A token for repository `name` in `ns`, valid for [`TOKEN_TTL`] seconds.
    ///
    /// # Errors
    ///
    /// A message naming the call that failed; never the token.
    pub fn token(&self, ns: &str, name: &str, scope: Scope) -> Result<String, String> {
        let (scope, at_ns, at_repo) = (
            scope.word(),
            format!("{}/{ns}/tokens", self.namespaces()),
            format!("{}/{name}/tokens", self.repos(ns)),
        );
        let v = self.api.post_any(&[
            (
                at_ns.clone(),
                json!({ "repo": name, "scope": scope, "ttl": TOKEN_TTL }),
            ),
            (at_ns.clone(), json!({ "repo": name, "scope": scope })),
            (
                at_ns.clone(),
                json!({ "name": name, "scope": scope, "ttl": TOKEN_TTL }),
            ),
            (
                at_ns,
                json!({ "repository": name, "scope": scope, "ttl": TOKEN_TTL }),
            ),
            (at_repo.clone(), json!({ "scope": scope, "ttl": TOKEN_TTL })),
            (at_repo, json!({ "scope": scope })),
        ])?;
        token_in(&v).ok_or_else(|| {
            format!(
                "repository `{name}`: no token in the API's answer, whose fields are {}",
                shape(&v)
            )
        })
    }
}

/// Where a token may sit in an answer, by field name.
const TOKEN_FIELDS: [&str; 6] = [
    "token",
    "plaintext",
    "value",
    "secret",
    "access_token",
    "password",
];

/// The token in an answer: the answer itself, a token field, or one inside `token` or `data`.
fn token_in(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    TOKEN_FIELDS
        .iter()
        .find_map(|k| v[*k].as_str())
        .map(str::to_string)
        .or_else(|| {
            ["token", "data", "result"]
                .iter()
                .find_map(|k| v.get(*k).filter(|n| n.is_object()).and_then(token_in))
        })
}

/// An answer's field names and types, never its values.
fn shape(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let fields: Vec<String> = o
                .iter()
                .map(|(k, x)| match x {
                    Value::Object(_) => format!("{k}: {}", shape(x)),
                    Value::Array(_) => format!("{k}: array"),
                    Value::String(_) => format!("{k}: string"),
                    Value::Number(_) => format!("{k}: number"),
                    Value::Bool(_) => format!("{k}: bool"),
                    Value::Null => format!("{k}: null"),
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Value::Array(_) => "an array".into(),
        Value::Null => "empty".into(),
        _ => "a scalar".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{shape, token_in};
    use serde_json::json;

    #[test]
    fn a_token_is_found_where_the_api_puts_it_and_never_shown() {
        assert_eq!(token_in(&json!("t1")).as_deref(), Some("t1"));
        assert_eq!(
            token_in(&json!({ "plaintext": "t2" })).as_deref(),
            Some("t2")
        );
        assert_eq!(
            token_in(&json!({ "token": { "value": "t3" } })).as_deref(),
            Some("t3")
        );
        assert_eq!(
            token_in(&json!({ "data": { "access_token": "t4" } })).as_deref(),
            Some("t4")
        );
        assert_eq!(token_in(&json!({ "id": "x" })), None);
        let shown =
            shape(&json!({ "id": "secret-id", "nested": { "key": "secret-value" }, "n": 1 }));
        assert_eq!(shown, "{ id: string, n: number, nested: { key: string } }");
        assert!(!shown.contains("secret"));
    }
}
