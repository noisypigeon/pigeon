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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Create it.
    Create,
    /// Update it in place.
    Update,
    /// Destroy and recreate it.
    Replace,
    /// Destroy it.
    Delete,
    /// Move it to a new address, unchanged.
    Move,
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
            Self::Move => "move",
        }
    }
}

/// One resource change: an address, an action and, for an update, the names of the attributes
/// it changes; never values.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    /// The resource address.
    pub address: String,
    /// The action.
    pub action: Action,
    /// For an update, the top-level attributes that change or become unknown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<String>,
}

/// A unit's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPlan {
    /// The unit's path.
    pub unit: String,
    /// Its changes; empty when there are none.
    pub changes: Vec<Change>,
    /// A line to print beside its changes, e.g. pending credentials.
    pub note: Option<String>,
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
                Action::Move => (a, c, d),
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

/// What every `tofu` call in one unit needs.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// The state passphrase.
    pub passphrase: String,
    /// Extra environment, e.g. `TF_VAR_secrets`.
    pub vars: Vec<(String, String)>,
    /// Values never to print, beside the passphrase.
    pub hidden: Vec<String>,
    /// The repository lockfile, if providers are allowed.
    pub lockfile: Option<std::path::PathBuf>,
    /// The allowlisted sources this unit uses; only their lockfile blocks are copied.
    pub providers: Vec<String>,
    /// Plan without refreshing, when credentials are placeholders.
    pub no_refresh: bool,
}

impl Env {
    fn scrub(&self, text: &str) -> String {
        let mut out = redact(text, &self.passphrase);
        for h in self.hidden.iter().filter(|h| h.len() >= 4) {
            out = out.replace(h.as_str(), "***");
        }
        out
    }
}

/// Only each diagnostic's summary line, never its detail.
fn diagnostics(output: &Output, env: &Env) -> String {
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    let summaries: Vec<&str> = text
        .lines()
        .map(str::trim)
        // Each error, and the resource address it names (`with <address>,`).
        .filter(|l| l.starts_with("Error:") || l.starts_with("with "))
        .collect();
    let body = if summaries.is_empty() {
        "no diagnostic summary".to_string()
    } else {
        summaries.join("; ")
    };
    mask(&env.scrub(&body))
}

fn run(unit: &str, work: &Path, env: &Env, args: &[&str]) -> Result<Output, Error> {
    Command::new("tofu")
        .args(args)
        .current_dir(work)
        .env_remove(crate::PASSPHRASE_VAR)
        .env_remove(crate::CREDENTIALS_VAR)
        .env_remove(crate::secrets::PASSPHRASE_VAR)
        .env_remove(crate::secrets::WORKLOAD_PASSPHRASE_VAR)
        .env("TF_ENCRYPTION", encryption(&env.passphrase))
        .env("TF_IN_AUTOMATION", "1")
        .env("TF_INPUT", "0")
        .envs(env.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .map_err(|e| Error::Tofu {
            unit: unit.to_string(),
            message: format!("`tofu` could not start: {e}"),
        })
}

/// Runs one `tofu` step, naming it and its duration on stderr as it goes.
fn step(unit: &str, work: &Path, env: &Env, args: &[&str], ok: &[i32]) -> Result<Output, Error> {
    let started = std::time::Instant::now();
    eprintln!("cobot: {unit}: tofu {}…", args[0]);
    let output = run(unit, work, env, args)?;
    eprintln!(
        "cobot: {unit}: tofu {} took {:.1}s",
        args[0],
        started.elapsed().as_secs_f64()
    );
    let code = output.status.code().unwrap_or(-1);
    if ok.contains(&code) {
        Ok(output)
    } else {
        Err(Error::Tofu {
            unit: unit.to_string(),
            message: format!(
                "`tofu {}` exited {code}: {}",
                args[0],
                diagnostics(&output, env)
            ),
        })
    }
}

/// Parses `tofu show -json` output into changes.
fn action(change: &Value) -> Option<Action> {
    let actions: Vec<&str> = change["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    match actions.as_slice() {
        ["create"] => Some(Action::Create),
        ["update"] => Some(Action::Update),
        ["delete"] => Some(Action::Delete),
        ["delete", "create"] | ["create", "delete"] => Some(Action::Replace),
        _ => None,
    }
}

/// The top-level attribute names whose value differs between `before` and `after`, or which
/// become unknown; sorted, masked, never values.
fn changed(change: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let before = change["before"].as_object().unwrap_or(&empty);
    let after = change["after"].as_object().unwrap_or(&empty);
    let unknown = change["after_unknown"].as_object().unwrap_or(&empty);
    let mut names: Vec<String> = before
        .keys()
        .chain(after.keys())
        .chain(unknown.keys())
        .filter(|k| before.get(*k) != after.get(*k) || unknown.get(*k).is_some_and(any_true))
        .map(|k| mask(k))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Whether an `after_unknown` value marks anything unknown.
fn any_true(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Array(items) => items.iter().any(any_true),
        Value::Object(map) => map.values().any(any_true),
        _ => false,
    }
}

/// Parses `tofu show -json` output into resource changes, then root output changes.
fn changes(json: &str) -> Vec<Change> {
    let Ok(plan) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rc in plan["resource_changes"].as_array().into_iter().flatten() {
        let Some(address) = rc["address"].as_str() else {
            continue;
        };
        if let Some(from) = rc["previous_address"].as_str() {
            out.push(Change {
                address: mask(&format!("{from} → {address}")),
                action: Action::Move,
                attributes: Vec::new(),
            });
        }
        if let Some(action) = action(&rc["change"]) {
            let attributes = if action == Action::Update {
                changed(&rc["change"])
            } else {
                Vec::new()
            };
            out.push(Change {
                address: mask(address),
                action,
                attributes,
            });
        }
    }
    for (name, change) in plan["output_changes"].as_object().into_iter().flatten() {
        if let Some(action) = action(change) {
            out.push(Change {
                address: mask(&format!("output.{name}")),
                action,
                attributes: Vec::new(),
            });
        }
    }
    out
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Initializes: from the repository lockfile, read-only, when there is one;
/// otherwise with only built-in providers.
pub fn init(unit: &str, work: &Path, env: &Env) -> Result<(), Error> {
    let lock = work.join(".terraform.lock.hcl");
    if let Some(source) = env.lockfile.as_ref().filter(|_| !env.providers.is_empty()) {
        let text = fs::read_to_string(source).map_err(io(source))?;
        let sources: Vec<&str> = env.providers.iter().map(String::as_str).collect();
        fs::write(&lock, crate::providers::lock_for(&text, &sources)).map_err(io(&lock))?;
        step(
            unit,
            work,
            env,
            &[
                "init",
                "-input=false",
                "-no-color",
                "-reconfigure",
                "-lockfile=readonly",
            ],
            &[0],
        )?;
    } else {
        let _ = fs::remove_file(&lock);
        let plugins = work.join(".no-plugins");
        fs::create_dir_all(&plugins).map_err(io(&plugins))?;
        let plugin_dir = format!("-plugin-dir={}", plugins.display());
        step(
            unit,
            work,
            env,
            &[
                "init",
                "-input=false",
                "-no-color",
                "-reconfigure",
                &plugin_dir,
            ],
            &[0],
        )?;
    }
    Ok(())
}

/// The unit's current state, as `tofu show -json`.
pub fn show_state(unit: &str, work: &Path, env: &Env) -> Result<String, Error> {
    let shown = step(unit, work, env, &["show", "-json", "-no-color"], &[0])?;
    Ok(String::from_utf8_lossy(&shown.stdout).into_owned())
}

/// The plan file in the work directory as `tofu show -json`; it is decrypted with the state
/// passphrase, so its values are shown only after [`crate::plan_diff`] redacts them.
pub fn show_plan(unit: &str, work: &Path, env: &Env) -> Result<String, Error> {
    let shown = step(
        unit,
        work,
        env,
        &["show", "-json", "-no-color", PLAN_FILE],
        &[0],
    )?;
    Ok(String::from_utf8_lossy(&shown.stdout).into_owned())
}

/// The plan file in a unit's work directory.
pub fn plan_file(work: &Path) -> std::path::PathBuf {
    work.join(PLAN_FILE)
}

/// Plans to `plan.tfplan`; the work directory must already be initialized.
pub fn plan(unit: &str, work: &Path, env: &Env) -> Result<UnitPlan, Error> {
    let out = format!("-out={PLAN_FILE}");
    let mut args = vec![
        "plan",
        "-input=false",
        "-no-color",
        "-detailed-exitcode",
        &out,
    ];
    if env.no_refresh {
        args.push("-refresh=false");
    }
    let planned = step(unit, work, env, &args, &[0, 2])?;
    if planned.status.code() == Some(0) {
        return Ok(UnitPlan {
            unit: unit.to_string(),
            changes: Vec::new(),
            note: None,
        });
    }
    let shown = step(
        unit,
        work,
        env,
        &["show", "-json", "-no-color", PLAN_FILE],
        &[0],
    )?;
    Ok(UnitPlan {
        unit: unit.to_string(),
        changes: changes(&String::from_utf8_lossy(&shown.stdout)),
        note: None,
    })
}

/// Applies `plan.tfplan`.
pub fn apply(unit: &str, work: &Path, env: &Env) -> Result<(), Error> {
    step(
        unit,
        work,
        env,
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
            {"address":"c.d","change":{"actions":["delete","create"]}}],
            "output_changes":{"tokens":{"actions":["create"],"after":"secret"},"same":{"actions":["no-op"]}}}"#;
        let got = changes(json);
        assert_eq!(got.len(), 3);
        assert_eq!(got[2].address, "output.tokens");

        let moved = r#"{"resource_changes":[
            {"address":"b.x","previous_address":"a.x","change":{"actions":["update"]}},
            {"address":"d.y","previous_address":"c.y","change":{"actions":["no-op"]}}]}"#;
        let both = changes(moved);
        let words: Vec<(&str, &str)> = both
            .iter()
            .map(|c| (c.action.word(), c.address.as_str()))
            .collect();
        assert_eq!(
            words,
            [
                ("move", "a.x → b.x"),
                ("update", "b.x"),
                ("move", "c.y → d.y")
            ]
        );
        assert_eq!(got[0].address, "module.p.terraform_data.this");
        assert_eq!(got[1].action, Action::Replace);
        assert!(!format!("{got:?}").contains("secret"));
        assert!(got.iter().all(|c| c.attributes.is_empty()), "{got:?}");
    }

    #[test]
    fn an_update_names_its_changed_attributes_never_their_values() {
        let json = r#"{"resource_changes":[{"address":"module.d1.x.this","change":{
            "actions":["update"],
            "before":{"name":"db","hint":"weur","replication":null,"token":"old-secret"},
            "after":{"name":"db","hint":"weur","replication":null,"token":"new-secret"},
            "after_unknown":{"replication":{"mode":true},"version":true,"name":false}}}]}"#;
        let got = changes(json);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].attributes, ["replication", "token", "version"]);
        let printed = format!("{got:?}");
        assert!(
            !printed.contains("secret") && !printed.contains("weur"),
            "{printed}"
        );
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
