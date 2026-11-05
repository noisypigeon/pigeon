//! Artifacts' answers, read the same way by the CLI and the Worker: where a token and a remote
//! sit; see `.claude/decisions/2026-11-05-development-git-hosting-cloudflare-artifacts.md`.

use serde_json::Value;

/// How long a minted git token lives, in seconds.
pub const TOKEN_TTL: u64 = 900;

/// The remote in a repository's answer.
#[must_use]
pub fn remote_in(v: &Value) -> Option<String> {
    ["remote", "url", "git_url"]
        .iter()
        .find_map(|k| v[*k].as_str())
        .map(str::to_string)
}

/// The token-minting body for repository `repo`, read-only, valid for `ttl` seconds.
#[must_use]
pub fn read_token_body(repo: &str, ttl: u64) -> Value {
    serde_json::json!({ "repo": repo, "scope": "read", "ttl": ttl })
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
#[must_use]
pub fn token_in(v: &Value) -> Option<String> {
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
#[must_use]
pub fn shape(v: &Value) -> String {
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
