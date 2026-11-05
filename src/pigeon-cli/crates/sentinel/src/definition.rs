//! Platform-defined policies an instance invokes with parameters, per the control-plane ADR's
//! "A run's instance root, built from D1" amendment §2
//! (`.claude/decisions/2026-11-05-runtime-control-plane-worker.md`).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::policy::Policy;

/// A parameter's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// One string.
    String,
    /// A list of strings.
    List,
}

/// One declared parameter.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// Its type.
    #[serde(rename = "type")]
    pub kind: Kind,
    /// One line saying what it is.
    pub description: String,
    /// Its value when an invocation omits it; without one, it is required.
    pub default: Option<Value>,
}

/// A policy with `params`, instantiated only through an [`Invocation`].
#[derive(Debug, Clone)]
pub struct Definition {
    /// Its parameters, by name.
    pub params: BTreeMap<String, Param>,
    body: Value,
}

/// An instance's use of a definition: its name, and its parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    /// The definition's name.
    #[serde(rename = "use")]
    pub name: String,
    /// Its parameters.
    #[serde(default)]
    pub with: Map<String, Value>,
}

impl Definition {
    /// Parses a definition: a policy, optionally with `params`, whose strings may hold
    /// `${{ name }}` or `${{ name | regex }}`.
    ///
    /// # Errors
    ///
    /// A message naming what is invalid, including a parameter declared wrong or a placeholder
    /// naming none.
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let mut body: Value = serde_saphyr::from_str(yaml).map_err(|e| e.to_string())?;
        let params = match body.as_object_mut().and_then(|o| o.remove("params")) {
            Some(p) => serde_json::from_value::<BTreeMap<String, Param>>(p)
                .map_err(|e| format!("params: {e}"))?,
            None => BTreeMap::new(),
        };
        for (name, param) in &params {
            if param.description.trim().is_empty() || param.description.contains('\n') {
                return Err(format!(
                    "params.{name}: description must be one non-empty line"
                ));
            }
            if let Some(d) = &param.default {
                check(name, param.kind, d)?;
            }
        }
        let definition = Self { params, body };
        let probe = definition
            .params
            .iter()
            .map(|(name, p)| (name.clone(), sample(p.kind)))
            .collect();
        definition.build(&probe)?;
        Ok(definition)
    }

    /// The policy `invocation` asks for.
    ///
    /// # Errors
    ///
    /// A message naming an unknown, missing or mistyped parameter, or what the resulting
    /// policy gets wrong.
    pub fn invoke(&self, invocation: &Invocation) -> Result<Policy, String> {
        if let Some(unknown) = invocation
            .with
            .keys()
            .find(|k| !self.params.contains_key(*k))
        {
            return Err(format!("unknown parameter `{unknown}`"));
        }
        let mut values = BTreeMap::new();
        for (name, param) in &self.params {
            let value = invocation
                .with
                .get(name)
                .or(param.default.as_ref())
                .ok_or_else(|| format!("parameter `{name}` is required"))?;
            check(name, param.kind, value)?;
            values.insert(name.clone(), value.clone());
        }
        self.build(&values)
    }

    fn build(&self, values: &BTreeMap<String, Value>) -> Result<Policy, String> {
        let body = substitute(&self.body, values)?;
        let policy: Policy = serde_json::from_value(body).map_err(|e| e.to_string())?;
        policy.validate()?;
        Ok(policy)
    }
}

fn check(name: &str, kind: Kind, value: &Value) -> Result<(), String> {
    let ok = match kind {
        Kind::String => value.is_string(),
        Kind::List => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "parameter `{name}` must be {}",
            match kind {
                Kind::String => "a string",
                Kind::List => "a list of strings",
            }
        ))
    }
}

fn sample(kind: Kind) -> Value {
    match kind {
        Kind::String => Value::String("x".into()),
        Kind::List => Value::Array(vec![Value::String("x".into())]),
    }
}

fn substitute(value: &Value, values: &BTreeMap<String, Value>) -> Result<Value, String> {
    Ok(match value {
        Value::String(s) => interpolate(s, values)?,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| substitute(v, values))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), substitute(v, values)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

/// A string with its placeholders filled. A string that is one bare placeholder becomes the
/// value itself, so a list stays a list.
fn interpolate(s: &str, values: &BTreeMap<String, Value>) -> Result<Value, String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        let end = rest[start..]
            .find("}}")
            .map(|e| start + e)
            .ok_or_else(|| format!("unclosed placeholder in `{s}`"))?;
        let (name, filter) = match rest[start + 3..end].split_once('|') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (rest[start + 3..end].trim(), None),
        };
        let value = values
            .get(name)
            .ok_or_else(|| format!("placeholder `{name}` names no parameter"))?;
        if start == 0 && end + 2 == rest.len() && out.is_empty() && filter.is_none() {
            return Ok(value.clone());
        }
        out.push_str(&rest[..start]);
        match (filter, value) {
            (None, Value::String(v)) => out.push_str(v),
            (Some("regex"), Value::String(v)) => out.push_str(&regex::escape(v)),
            (Some("regex"), Value::Array(items)) => {
                let alternatives: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(regex::escape)
                    .collect();
                let _ = write!(out, "(?:{})", alternatives.join("|"));
            }
            (None, _) => {
                return Err(format!(
                    "list `{name}` fits only a whole value or `| regex`"
                ));
            }
            (Some(f), _) => return Err(format!("unknown filter `{f}`")),
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

#[cfg(test)]
mod tests {
    use super::{Definition, Invocation};

    const DEF: &str = "description: Workloads sit only under the instance's zones.\nparams:\n  zones: { type: list, description: The zones. }\n  dir: { type: string, description: The workloads directory., default: workloads }\ntarget: tree\nrules:\n  - find:\n      match: '^${{ dir | regex }}/'\n      allow_under: ${{ zones }}\n      forbid_content: 'zone: ${{ zones | regex }}$'\n";

    fn invoke(with: &str) -> Result<super::Policy, String> {
        let definition = Definition::parse(DEF).expect("definition parses");
        let invocation: Invocation =
            serde_saphyr::from_str(&format!("use: zones\nwith: {with}\n")).expect("invocation");
        definition.invoke(&invocation)
    }

    #[test]
    fn an_invocation_fills_whole_values_strings_and_escaped_regexes() {
        let policy = invoke("{ zones: [a.com, b.org] }").expect("valid");
        let crate::policy::Rule::Find(find) = &policy.rules[0] else {
            panic!("a find rule");
        };
        assert_eq!(find.matches.0.as_str(), "^workloads/");
        assert_eq!(find.allow_under, ["a.com", "b.org"]);
        let content = find.forbid_content.as_ref().expect("content");
        assert_eq!(content.0.as_str(), r"zone: (?:a\.com|b\.org)$");
        assert!(!content.0.is_match("zone: aXcom"), "dots are literal");
    }

    #[test]
    fn invocations_are_checked_against_the_declared_params() {
        assert_eq!(
            invoke("{}").expect_err("missing"),
            "parameter `zones` is required"
        );
        assert_eq!(
            invoke("{ zones: [a], extra: x }").expect_err("unknown"),
            "unknown parameter `extra`"
        );
        assert_eq!(
            invoke("{ zones: a }").expect_err("mistyped"),
            "parameter `zones` must be a list of strings"
        );
        assert_eq!(
            invoke("{ zones: [a], dir: [b] }").expect_err("mistyped"),
            "parameter `dir` must be a string"
        );
        assert!(invoke("{ zones: [a], dir: other }").is_ok());
    }

    #[test]
    fn a_bad_definition_is_refused_when_it_loads() {
        for (yaml, want) in [
            (
                "description: x\ntarget: { file: f }\nrules:\n  - contains: { file: f, text: '${{ nope }}' }\n",
                "names no parameter",
            ),
            (
                "description: x\nparams:\n  l: { type: list, description: L. }\ntarget: { file: f }\nrules:\n  - contains: { file: f, text: 'a ${{ l }}' }\n",
                "fits only a whole value",
            ),
            (
                "description: x\nparams:\n  s: { type: string, description: S., default: [a] }\ntarget: { file: f }\nrules: []\n",
                "must be a string",
            ),
            (
                "description: x\nparams:\n  s: { type: number, description: S. }\ntarget: { file: f }\nrules: []\n",
                "params",
            ),
            (
                "description: x\ntarget: { file: f }\nrules:\n  - contains: { file: f, text: '${{ s' }\n",
                "unclosed",
            ),
        ] {
            let err = Definition::parse(yaml).expect_err(yaml);
            assert!(err.contains(want), "{yaml}: {err}");
        }
    }
}
