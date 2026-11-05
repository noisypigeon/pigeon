//! The operation catalog and its wire form, shared by the `engines` and `worker` builds
//! (control-plane ADR §1).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Error;

/// Who runs an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Backplane itself.
    Backplane,
    /// Policy and its enforcement.
    Sentinel,
    /// Actions: infrastructure, releases.
    Cobot,
    /// Planned: scheduling and data operations.
    Gantry,
}

impl Engine {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Backplane => "backplane",
            Self::Sentinel => "sentinel",
            Self::Cobot => "cobot",
            Self::Gantry => "gantry",
        }
    }
}

/// What an operation may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Reads only.
    Read,
    /// Computes changes without making them.
    Plan,
    /// Makes changes.
    Apply,
}

impl Effect {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Plan => "plan",
            Self::Apply => "apply",
        }
    }
}

/// One operation a surface may request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Operation {
    /// Its dotted name, `<area>.<verb>`.
    pub name: &'static str,
    /// Who runs it.
    pub engine: Engine,
    /// What it may change.
    pub effect: Effect,
    /// What it does.
    pub summary: &'static str,
}

const CATALOG: &[Operation] = &[
    Operation {
        name: "instance.show",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Show the instance manifest",
    },
    Operation {
        name: "architecture.describe",
        engine: Engine::Backplane,
        effect: Effect::Read,
        summary: "Describe the instance's architecture as a graph and its layout",
    },
    Operation {
        name: "policy.check",
        engine: Engine::Sentinel,
        effect: Effect::Read,
        summary: "Check the instance against its policies (args: title?, body?, number?, files?, which add the pull-request policy)",
    },
    Operation {
        name: "infra.validate",
        engine: Engine::Cobot,
        effect: Effect::Read,
        summary: "Validate every infrastructure and workload unit",
    },
    Operation {
        name: "sites.build",
        engine: Engine::Cobot,
        effect: Effect::Read,
        summary: "Build static sites into the work directory with the instance's architecture as data; publishes nothing (args: sites?)",
    },
    Operation {
        name: "infra.plan",
        engine: Engine::Cobot,
        effect: Effect::Plan,
        summary: "Plan units and store the plan for 24 hours (args: units?; state, which serve supplies)",
    },
    Operation {
        name: "infra.apply",
        engine: Engine::Cobot,
        effect: Effect::Apply,
        summary: "Apply a stored, unexpired plan exactly (args: plan; state, which serve supplies)",
    },
];

/// The arguments an operation takes: each name, and whether it is required.
fn arguments(name: &str) -> &'static [(&'static str, bool)] {
    match name {
        "policy.check" => &[
            ("title", false),
            ("body", false),
            ("number", false),
            ("files", false),
        ],
        "sites.build" => &[("sites", false)],
        "infra.plan" => &[("state", true), ("units", false)],
        "infra.apply" => &[("state", true), ("plan", true)],
        _ => &[],
    }
}

/// Every operation.
#[must_use]
pub fn catalog() -> &'static [Operation] {
    CATALOG
}

/// A request for one operation.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The operation's name.
    pub operation: String,
    /// Its arguments, as [`catalog`]'s summaries name them.
    #[serde(default)]
    pub args: Map<String, Value>,
}

/// An operation's outcome; the wire form every surface shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The operation's name.
    pub operation: String,
    /// Whether it found no violation.
    pub ok: bool,
    /// What it returned.
    pub result: Value,
    /// Every violation found.
    pub violations: Vec<String>,
}

/// Checks a request's operation, argument names and argument types without running it.
///
/// # Errors
///
/// [`Error::Invalid`] naming every problem.
pub fn validate(request: &Request) -> Result<&'static Operation, Error> {
    let Some(op) = CATALOG.iter().find(|o| o.name == request.operation) else {
        return Err(Error::Invalid(vec![format!(
            "no operation `{}`",
            request.operation
        )]));
    };
    let allowed = arguments(op.name);
    let mut problems: Vec<String> = request
        .args
        .keys()
        .filter(|k| !allowed.iter().any(|(a, _)| a == k))
        .map(|k| format!("`{}` takes no argument `{k}`", op.name))
        .collect();
    problems.extend(
        allowed
            .iter()
            .filter(|(a, required)| *required && !request.args.contains_key(*a))
            .map(|(a, _)| format!("`{}` needs argument `{a}`", op.name)),
    );
    for (name, value) in &request.args {
        let ok = match name.as_str() {
            "state" | "plan" => value.is_string(),
            "title" | "body" | "number" | "files" => {
                value.is_string() || value.is_number() || value.is_boolean()
            }
            "units" | "sites" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            _ => true,
        };
        if !ok {
            problems.push(format!(
                "argument `{name}` must be {}",
                match name.as_str() {
                    "units" | "sites" => "a list of strings",
                    "state" | "plan" => "a string",
                    _ => "text",
                }
            ));
        }
    }
    if problems.is_empty() {
        Ok(op)
    } else {
        Err(Error::Invalid(problems))
    }
}

#[cfg(test)]
mod tests {
    use super::Response;
    use serde_json::json;

    #[test]
    fn a_response_round_trips_its_wire_form() {
        let sent = Response {
            operation: "infra.plan".into(),
            ok: false,
            result: json!({ "plan": "0123", "units": [] }),
            violations: vec!["a violation".into()],
        };
        let text = serde_json::to_string(&sent).expect("serialize");
        let back: Response = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, sent);
    }
}
