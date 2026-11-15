//! What starts a session's tooling: an executor's `create_session` arguments, and main's
//! `pigeon` build; see `.claude/decisions/2026-11-15-development-no-python-rust-tooling.md`
//! §1 and §2.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;

/// The refusal when `config.yaml` names no `repository`.
pub const NO_REPOSITORY: &str =
    "config.yaml names no `repository`, so an executor would start with no checkout";

/// The executor's prompt; `{model}`, `{brief}`, `{revision}`, `{planner}` and
/// `{planner_session}` are filled in.
const EXECUTOR: &str = "You are an executor session for pigeon, of class {model}, carrying out brief {brief} (revision {revision}), which planner session {planner} wrote. The brief has made every decision: follow it exactly, and CLAUDE.md's \"Briefs and model classes\" section.

0. Check first: the working directory is a git checkout of pigeon holding CLAUDE.md, and ToolSearch finds `session_start`. If either is missing, `send_message` your planner's session, `{planner_session}`, saying which is missing, and stop. Never `add_repo`, clone, or work around a missing tool.
1. Per CLAUDE.md: add the `artifacts` remote, fetch its `main`, and branch from `artifacts/main`.
2. `session_start` with this session's URL as `label`, `model` \"{model}\", `planner` \"{planner}\" and `brief` \"{brief}\". Write {\"label\", \"ticket\"} to ~/.config/pigeon/session.json, then arm `Monitor` with `sh tooling/session/session.sh follow`.
3. `brief_show` {brief}, then `task_list` and `task_claim` the `brief` task for {brief}.
4. `change_propose` with a slug, `brief` \"{brief}\", `session` and `ticket`; the brief writes its title, plan and scope. Authorize it as CLAUDE.md says.
5. Do the brief's steps in order, and only them, touching only each step's files. After each step, check its `done_when`; if it fails, try once more within the step. Run the brief's `checks` before every push, and push as CLAUDE.md says.
6. On a second failure, a `stop_if`, a contradiction, or anything the brief doesn't cover: stop, and `brief_escalate` your change with the `question` and what you `tried`. Your branch takes no push until the planner answers; wait for it. Never work around it, change the brief's decisions, or widen the scope.
7. Never write or amend an ADR, and never call an owner operation. When your change merges, or the planner takes the brief back, `session_end`.";

/// `create_session`'s arguments for an executor, in the order they print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Executor {
    /// The class's model ID.
    pub model: String,
    /// The session's title.
    pub title: String,
    /// Its first message.
    pub prompt: String,
    /// The repository it starts with checked out.
    pub source_url: String,
    /// Its tags.
    pub tags: Vec<String>,
}

fn plain(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// `config.yaml`'s `repository`, from its text.
#[must_use]
pub fn repository(config: &str) -> Option<String> {
    config
        .lines()
        .find_map(|row| row.strip_prefix("repository:"))
        .map(|v| v.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .filter(|v| !v.is_empty())
}

/// The arguments for an executor of `brief`, `brief.show`'s answer for `want`, with `models`
/// (`models.json`), the repository `source` and `extra` tags; else why not.
///
/// # Errors
///
/// An unknown brief, or one whose class no executor carries out.
pub fn executor(
    want: &str,
    brief: &Value,
    models: &Value,
    source: &str,
    extra: &[String],
) -> Result<Executor, String> {
    let revisions = brief["revisions"].as_array().filter(|r| !r.is_empty());
    let Some(revisions) = revisions else {
        return Err(format!("no brief `{want}`"));
    };
    let id = plain(&brief["id"]);
    let draft = &revisions[revisions.len() - 1]["draft"];
    let model = draft["model"].as_str().unwrap_or_default();
    let ids = models[model]
        .as_str()
        .filter(|_| matches!(model, "haiku" | "sonnet"));
    let Some(ids) = ids else {
        return Err(format!(
            "brief {id} is for `{model}`, which no executor carries out"
        ));
    };
    let planner = brief["planner"].as_str().unwrap_or_default();
    let planner_session = planner
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let prompt = EXECUTOR
        .replace("{model}", model)
        .replace("{brief}", &id)
        .replace("{revision}", &revisions.len().to_string())
        .replace("{planner_session}", planner_session)
        .replace("{planner}", planner);
    let mut tags = vec![format!("pigeon:brief-{id}")];
    let adr = draft["adr"].as_str().unwrap_or_default();
    if let Some(stem) = adr.rsplit('/').next().and_then(|a| a.strip_suffix(".md")) {
        tags.push(format!("pigeon:adr-{stem}"));
    }
    for t in extra {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    Ok(Executor {
        model: ids.to_string(),
        title: format!(
            "pigeon {id} ({model}): {}",
            draft["title"].as_str().unwrap_or_default()
        ),
        prompt,
        source_url: source.to_string(),
        tags,
    })
}

/// `deploy.status`'s `latest_build`: its commit (40 hex) and SHA-256 (64 hex), when both are.
#[must_use]
pub fn latest_build(status: &Value) -> Option<(String, String)> {
    let b = &status["pipeline"]["latest_build"];
    let hex = |v: &Value, n: usize| {
        v.as_str()
            .filter(|s| s.len() == n && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase)
    };
    Some((hex(&b["sha"], 40)?, hex(&b["sha256"], 64)?))
}

/// The SHA-256 of `bytes`, in lowercase hex.
#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Sha256::digest(bytes)
        .iter()
        .flat_map(|b| [HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]])
        .map(char::from)
        .collect()
}

/// Whether `bytes` have the SHA-256 `want`.
#[must_use]
pub fn verified(bytes: &[u8], want: &str) -> bool {
    sha256(bytes).eq_ignore_ascii_case(want)
}
