//! What a change asks the owner at its plan approval; see
//! `.claude/decisions/2026-11-13-security-approvals-ask-owner-inputs.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most asks a change holds.
pub const MAX_ASKS: usize = 8;
/// The longest answer.
pub const MAX_LEN: usize = 256;

/// The characters an answer may hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chars {
    /// `0-9a-f`.
    Hex,
    /// `a-z`.
    Lower,
    /// `A-Za-z0-9`.
    Alnum,
    /// `A-Za-z0-9._:=+/-`.
    #[default]
    Token,
}

impl Chars {
    /// Whether `c` is in the set.
    #[must_use]
    pub fn allows(self, c: char) -> bool {
        match self {
            Self::Hex => c.is_ascii_digit() || ('a'..='f').contains(&c),
            Self::Lower => c.is_ascii_lowercase(),
            Self::Alnum => c.is_ascii_alphanumeric(),
            Self::Token => c.is_ascii_alphanumeric() || "._:=+/-".contains(c),
        }
    }

    /// The set as a regex character class, for an HTML `pattern` attribute.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Hex => "[0-9a-f]",
            Self::Lower => "[a-z]",
            Self::Alnum => "[A-Za-z0-9]",
            Self::Token => r"[A-Za-z0-9._:=+/\-]",
        }
    }
}

fn one() -> usize {
    1
}

fn most() -> usize {
    MAX_LEN
}

/// One question for the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    /// Its key in `answers`.
    pub key: String,
    /// What the field is called.
    pub label: String,
    /// Where to find the value.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    /// The characters allowed.
    #[serde(default)]
    pub chars: Chars,
    /// The shortest answer.
    #[serde(default = "one")]
    pub min: usize,
    /// The longest answer.
    #[serde(default = "most")]
    pub max: usize,
}

impl Ask {
    /// The HTML `pattern` attribute an answer matches.
    #[must_use]
    pub fn pattern(&self) -> String {
        format!("{}{{{},{}}}", self.chars.class(), self.min, self.max)
    }

    /// Whether `value` answers it.
    #[must_use]
    pub fn admits(&self, value: &str) -> bool {
        let n = value.chars().count();
        (self.min..=self.max).contains(&n) && value.chars().all(|c| self.chars.allows(c))
    }
}

fn key_ok(key: &str) -> bool {
    let mut cs = key.chars();
    key.len() <= 40
        && cs.next().is_some_and(|c| c.is_ascii_lowercase())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `change.propose`'s `asks`, as JSON text.
///
/// # Errors
///
/// When it isn't a list of valid asks.
pub fn parse(text: &str) -> Result<Vec<Ask>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let asks: Vec<Ask> =
        serde_json::from_str(text).map_err(|e| format!("`asks` is not a list of asks: {e}"))?;
    if asks.len() > MAX_ASKS {
        return Err(format!("a change asks at most {MAX_ASKS} questions"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for a in &asks {
        if !key_ok(&a.key) {
            return Err(format!(
                "ask key `{}` must be lowercase letters, digits and `_`, starting with a letter, at most 40",
                a.key
            ));
        }
        if !seen.insert(&a.key) {
            return Err(format!("ask key `{}` is asked twice", a.key));
        }
        if a.label.trim().is_empty() || a.label.chars().count() > 80 {
            return Err(format!(
                "ask `{}`'s label must be 1 to 80 characters",
                a.key
            ));
        }
        if a.help.chars().count() > 300 {
            return Err(format!("ask `{}`'s help is over 300 characters", a.key));
        }
        if !(1 <= a.min && a.min <= a.max && a.max <= MAX_LEN) {
            return Err(format!("ask `{}` needs 1 ≤ min ≤ max ≤ {MAX_LEN}", a.key));
        }
    }
    Ok(asks)
}

/// Checks `answers` against `asks`; problems name keys, never values.
///
/// # Errors
///
/// A missing, extra or inadmissible answer.
pub fn check(asks: &[Ask], answers: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(extra) = answers.keys().find(|k| !asks.iter().any(|a| &a.key == *k)) {
        return Err(format!("`{extra}` was not asked"));
    }
    for a in asks {
        match answers.get(&a.key) {
            None => return Err(format!("`{}` is not answered", a.key)),
            Some(v) if !a.admits(v) => {
                return Err(format!(
                    "`{}` must be {} to {} characters of {}",
                    a.key,
                    a.min,
                    a.max,
                    a.chars.class()
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// The answers in an operation's `answers` argument: an object of strings, or its JSON text.
#[must_use]
pub fn answers_of(v: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    let v = match v {
        Some(serde_json::Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
        Some(v) => v.clone(),
        None => serde_json::Value::Null,
    };
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect()
}
