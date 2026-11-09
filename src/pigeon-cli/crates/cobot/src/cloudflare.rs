//! A run's own Cloudflare token: the account, zone IDs and token ID it sees, for workloads that
//! use the catalogs directly, per
//! `.claude/decisions/2026-11-05-development-pigeon-its-own-instance.md` §4.

use std::time::Duration;

use serde_json::Value;

/// The environment variable holding a run's Cloudflare token.
pub const TOKEN_VAR: &str = "CLOUDFLARE_API_TOKEN";

/// The Cloudflare API's base URL.
pub const API: &str = "https://api.cloudflare.com/client/v4";

/// The run's token: `COBOT_CREDENTIALS`' `cloudflare.api_token`, else [`TOKEN_VAR`].
#[must_use]
pub fn token(credentials: Option<&str>, env: Option<String>) -> Option<String> {
    credentials
        .and_then(|c| serde_json::from_str::<Value>(c).ok())
        .and_then(|c| {
            c["cloudflare"]["api_token"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        })
        .or(env.filter(|t| !t.is_empty()))
}

/// The API, called with one token.
pub struct Api {
    base: String,
    token: String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Api {
    /// The API at `base` (default [`API`]), with `token`.
    #[must_use]
    pub fn new(base: Option<&str>, token: String) -> Self {
        Self {
            base: base.unwrap_or(API).trim_end_matches('/').to_string(),
            token,
        }
    }

    /// The token itself, for the providers and the handoff.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// `GET path`'s result.
    pub(crate) fn get(&self, path: &str) -> Result<Value, String> {
        self.call("GET", path, None)
            .and_then(|(status, body)| ok(status, &body, "GET", path))
    }

    /// `GET path`'s result, or `None` when it is 404.
    pub(crate) fn find(&self, path: &str) -> Result<Option<Value>, String> {
        match self.call("GET", path, None)? {
            (404, _) => Ok(None),
            (status, body) => ok(status, &body, "GET", path).map(Some),
        }
    }

    /// `DELETE path`; its result.
    pub(crate) fn delete(&self, path: &str) -> Result<Value, String> {
        self.call("DELETE", path, None)
            .and_then(|(status, b)| ok(status, &b, "DELETE", path))
    }

    /// `POST path` with a JSON `body`; its result.
    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.call("POST", path, Some(body))
            .and_then(|(status, b)| ok(status, &b, "POST", path))
    }

    /// `POST path` with the first of `bodies` the API accepts, trying the next on a 400 or 404;
    /// see `.claude/decisions/2026-11-05-development-git-hosting-cloudflare-artifacts.md`.
    pub(crate) fn post_first(&self, path: &str, bodies: &[Value]) -> Result<Value, String> {
        let tries: Vec<(String, Value)> = bodies
            .iter()
            .map(|b| (path.to_string(), b.clone()))
            .collect();
        self.post_any(&tries)
    }

    /// `POST` of the first `(path, body)` the API accepts, trying the next on a 400 or 404; the
    /// last failure when none does.
    pub(crate) fn post_any(&self, tries: &[(String, Value)]) -> Result<Value, String> {
        let mut last = "POST: nothing to send".to_string();
        for (path, b) in tries {
            match self.call("POST", path, Some(b)) {
                Ok((status @ (400 | 404), body)) => {
                    last = ok(status, &body, "POST", path).err().unwrap_or_default();
                }
                Ok((status, body)) => return ok(status, &body, "POST", path),
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// `GET path`'s result, with extra request `headers`.
    pub(crate) fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
        self.call_with("GET", path, None, headers)
            .and_then(|(status, body)| ok(status, &body, "GET", path))
    }

    /// One call: its status and JSON body. Messages name the method and path, never the token.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
        self.call_with(method, path, body, &[])
    }

    fn call_with(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> Result<(u16, Value), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let what = shown(path);
        let url = format!("{}{path}", self.base);
        let auth = format!("Bearer {}", self.token);
        let sent = match (method, body) {
            ("DELETE", _) => agent.delete(&url).header("authorization", &auth).call(),
            (_, None) => headers
                .iter()
                .fold(
                    agent.get(&url).header("authorization", &auth),
                    |r, (k, v)| r.header(*k, *v),
                )
                .call(),
            (_, Some(b)) => agent
                .post(&url)
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .send(b.to_string().as_bytes()),
        };
        let mut r = sent.map_err(|_| format!("{method} {what}: the request failed"))?;
        let status = r.status().as_u16();
        let body: Value = r
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        Ok((status, body))
    }

    /// The one account the token sees.
    ///
    /// # Errors
    ///
    /// A message when the call fails or the token sees no account or more than one.
    pub fn account(&self) -> Result<String, String> {
        let accounts = self.get("/accounts?per_page=2")?;
        match accounts.as_array().map(Vec::as_slice) {
            Some([one]) => one["id"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "GET /accounts: no id".into()),
            Some([]) => Err("the token sees no account".into()),
            _ => Err("the token sees more than one account; scope it to one".into()),
        }
    }

    /// The ID of zone `name`.
    ///
    /// # Errors
    ///
    /// A message when the call fails or the token does not see exactly that zone.
    pub fn zone_id(&self, name: &str) -> Result<String, String> {
        let zones = self.get(&format!("/zones?name={name}&per_page=2"))?;
        match zones.as_array().map(Vec::as_slice) {
            Some([one]) if one["name"] == name => one["id"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "GET /zones: no id".into()),
            _ => Err(format!("the token does not see zone `{name}`")),
        }
    }

    /// The token's own ID, R2's S3 access key; a user token, else one owned by `account`.
    ///
    /// # Errors
    ///
    /// A message when neither verification answers.
    pub fn token_id(&self, account: &str) -> Result<String, String> {
        let id = |v: Value| v["id"].as_str().map(str::to_string);
        self.get("/user/tokens/verify")
            .ok()
            .and_then(id)
            .or_else(|| {
                self.get(&format!("/accounts/{account}/tokens/verify"))
                    .ok()
                    .and_then(id)
            })
            .ok_or_else(|| "the token could not be verified".into())
    }
}

/// `path` as messages show it: no query, and no account ID.
fn shown(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    match path.strip_prefix("/accounts/") {
        Some(rest) => match rest.split_once('/') {
            Some((_, tail)) => format!("/accounts/…/{tail}"),
            None => "/accounts/…".into(),
        },
        None => path.to_string(),
    }
}

/// A successful call's `result`, else a message naming the call and the API's own error messages.
fn ok(status: u16, body: &Value, method: &str, path: &str) -> Result<Value, String> {
    let what = shown(path);
    if (200..300).contains(&status) && body["success"] == true {
        return Ok(body["result"].clone());
    }
    let errors: Vec<&str> = body["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["message"].as_str())
        .collect();
    if errors.is_empty() {
        Err(format!("{method} {what}: status {status}"))
    } else {
        Err(format!(
            "{method} {what}: status {status}: {}",
            errors.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{shown, token};

    #[test]
    fn messages_never_show_an_account_id() {
        assert_eq!(
            shown("/accounts/0123456789abcdef0123456789abcdef/artifacts/namespaces?x=1"),
            "/accounts/…/artifacts/namespaces"
        );
        assert_eq!(shown("/accounts/0123456789abcdef"), "/accounts/…");
        assert_eq!(shown("/accounts?per_page=2"), "/accounts");
        assert_eq!(shown("/zones?name=a.com"), "/zones");
    }

    #[test]
    fn the_runner_credential_wins_over_the_environment() {
        let creds = r#"{"cloudflare":{"api_token":"from-runner"}}"#;
        assert_eq!(
            token(Some(creds), Some("from-env".into())).as_deref(),
            Some("from-runner")
        );
        assert_eq!(
            token(Some("{}"), Some("from-env".into())).as_deref(),
            Some("from-env")
        );
        assert_eq!(
            token(
                Some(r#"{"cloudflare":{"api_token":""}}"#),
                Some("from-env".into())
            )
            .as_deref(),
            Some("from-env"),
            "an empty runner token falls back"
        );
        assert_eq!(token(None, Some(String::new())), None);
        assert_eq!(token(None, None), None);
    }
}
