//! Runs `tofu` with encrypted state, capturing and redacting its output.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::LazyLock;

use regex::Regex;

use serde_json::Value;

use crate::Error;

const PLAN_FILE: &str = "plan.tfplan";

/// A quoted string, which may hold a value.
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(?:[^"\\]|\\.)*""#).expect("valid regex"));

/// Masks quoted text, so instance keys and diagnostic values never print.
fn mask(text: &str) -> String {
    QUOTED.replace_all(text, "\"…\"").into_owned()
}

/// What `tofu` will do to a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Create it.
    Create,
    /// Update it in place.
    Update,
    /// Destroy and recreate it.
    Replace,
    /// Destroy it.
    Delete,
}

impl Action {
    /// The word used in summaries.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Replace => "replace",
            Self::Delete => "delete",
        }
    }
}

/// One resource change: an address and an action, never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The resource address.
    pub address: String,
    /// The action.
    pub action: Action,
}

/// A unit's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPlan {
    /// The unit's path.
    pub unit: String,
    /// Its changes; empty when there are none.
    pub changes: Vec<Change>,
}

impl UnitPlan {
    /// Whether applying would change anything.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Resources to add, change and destroy; a replacement counts as one add and one destroy.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        self.changes
            .iter()
            .fold((0, 0, 0), |(a, c, d), ch| match ch.action {
                Action::Create => (a + 1, c, d),
                Action::Update => (a, c + 1, d),
                Action::Replace => (a + 1, c, d + 1),
                Action::Delete => (a, c, d + 1),
            })
    }
}

/// The `TF_ENCRYPTION` configuration: pbkdf2 into AES-GCM, enforced for state and plans.
fn encryption(passphrase: &str) -> String {
    let quoted = serde_json::to_string(passphrase).expect("string serializes");
    format!(
        "key_provider \"pbkdf2\" \"cobot\" {{ passphrase = {} }}\n\
         method \"aes_gcm\" \"cobot\" {{ keys = key_provider.pbkdf2.cobot }}\n\
         state {{\n  method = method.aes_gcm.cobot\n  enforced = true\n}}\n\
         plan {{\n  method = method.aes_gcm.cobot\n  enforced = true\n}}\n",
        quoted.replace("${", "$${").replace("%{", "%%{")
    )
}

fn redact(text: &str, passphrase: &str) -> String {
    if passphrase.is_empty() {
        text.to_string()
    } else {
        text.replace(passphrase, "***")
    }
}

/// Only each diagnostic's summary line, never its detail.
fn diagnostics(output: &Output, passphrase: &str) -> String {
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    let summaries: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Error:"))
        .collect();
    let body = if summaries.is_empty() {
        "no diagnostic summary".to_string()
    } else {
        summaries.join("; ")
    };
    mask(&redact(&body, passphrase))
}

fn run(unit: &str, work: &Path, passphrase: &str, args: &[&str]) -> Result<Output, Error> {
    Command::new("tofu")
        .args(args)
        .current_dir(work)
        .env("TF_ENCRYPTION", encryption(passphrase))
        .env("TF_IN_AUTOMATION", "1")
        .env("TF_INPUT", "0")
        .output()
        .map_err(|e| Error::Tofu {
            unit: unit.to_string(),
            message: format!("`tofu` could not start: {e}"),
        })
}

fn step(
    unit: &str,
    work: &Path,
    passphrase: &str,
    args: &[&str],
    ok: &[i32],
) -> Result<Output, Error> {
    let output = run(unit, work, passphrase, args)?;
    let code = output.status.code().unwrap_or(-1);
    if ok.contains(&code) {
        Ok(output)
    } else {
        Err(Error::Tofu {
            unit: unit.to_string(),
            message: format!(
                "`tofu {}` exited {code}: {}",
                args[0],
                diagnostics(&output, passphrase)
            ),
        })
    }
}

/// Parses `tofu show -json` output into changes.
fn changes(json: &str) -> Vec<Change> {
    let Ok(plan) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rc in plan["resource_changes"].as_array().into_iter().flatten() {
        let actions: Vec<&str> = rc["change"]["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let action = match actions.as_slice() {
            ["create"] => Action::Create,
            ["update"] => Action::Update,
            ["delete"] => Action::Delete,
            ["delete", "create"] | ["create", "delete"] => Action::Replace,
            _ => continue,
        };
        if let Some(address) = rc["address"].as_str() {
            out.push(Change {
                address: mask(address),
                action,
            });
        }
    }
    out
}

/// Initializes with only built-in providers, then plans to `plan.tfplan`.
pub fn plan(unit: &str, work: &Path, passphrase: &str) -> Result<UnitPlan, Error> {
    let plugins = work.join(".no-plugins");
    fs::create_dir_all(&plugins).map_err(|e| Error::Io {
        path: plugins.clone(),
        message: e.to_string(),
    })?;
    let plugin_dir = format!("-plugin-dir={}", plugins.display());
    step(
        unit,
        work,
        passphrase,
        &[
            "init",
            "-input=false",
            "-no-color",
            "-reconfigure",
            &plugin_dir,
        ],
        &[0],
    )?;
    let planned = step(
        unit,
        work,
        passphrase,
        &[
            "plan",
            "-input=false",
            "-no-color",
            "-detailed-exitcode",
            &format!("-out={PLAN_FILE}"),
        ],
        &[0, 2],
    )?;
    if planned.status.code() == Some(0) {
        return Ok(UnitPlan {
            unit: unit.to_string(),
            changes: Vec::new(),
        });
    }
    let shown = step(
        unit,
        work,
        passphrase,
        &["show", "-json", "-no-color", PLAN_FILE],
        &[0],
    )?;
    Ok(UnitPlan {
        unit: unit.to_string(),
        changes: changes(&String::from_utf8_lossy(&shown.stdout)),
    })
}

/// Applies `plan.tfplan`.
pub fn apply(unit: &str, work: &Path, passphrase: &str) -> Result<(), Error> {
    step(
        unit,
        work,
        passphrase,
        &["apply", "-input=false", "-no-color", PLAN_FILE],
        &[0],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Action, changes, encryption, redact};

    #[test]
    fn changes_keep_addresses_and_actions_only() {
        let json = r#"{"resource_changes":[
            {"address":"module.p.terraform_data.this","change":{"actions":["create"],"after":{"input":"secret"}}},
            {"address":"a.b","change":{"actions":["no-op"]}},
            {"address":"c.d","change":{"actions":["delete","create"]}}]}"#;
        let got = changes(json);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].address, "module.p.terraform_data.this");
        assert_eq!(got[1].action, Action::Replace);
        assert!(!format!("{got:?}").contains("secret"));
    }

    #[test]
    fn encryption_quotes_and_escapes_the_passphrase() {
        let cfg = encryption("pa\"ss${x}word-0123456789");
        assert!(cfg.contains(r#"passphrase = "pa\"ss$${x}word-0123456789""#));
        assert!(cfg.contains("enforced = true"));
    }

    #[test]
    fn quoted_values_are_masked() {
        assert_eq!(
            super::mask(r#"module.m.x.y["s3cr\"et"] and Error: bad "value""#),
            r#"module.m.x.y["…"] and Error: bad "…""#
        );
        assert_eq!(super::mask("x.y[0]"), "x.y[0]");
    }

    #[test]
    fn redaction_hides_the_passphrase() {
        assert_eq!(
            redact("a sekret-passphrase b", "sekret-passphrase"),
            "a *** b"
        );
    }
}
