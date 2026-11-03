//! The operation catalog and its wire form.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{Engine, Error, Instance};

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
        name: "policy.check",
        engine: Engine::Sentinel,
        effect: Effect::Read,
        summary: "Check the instance against its policies",
    },
    Operation {
        name: "infra.validate",
        engine: Engine::Cobot,
        effect: Effect::Read,
        summary: "Validate every infrastructure and workload unit",
    },
];

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
    /// Its arguments; empty for every v0 operation.
    #[serde(default)]
    pub args: Map<String, Value>,
}

/// An operation's outcome; the wire form every surface shares.
#[derive(Debug, Clone, PartialEq, Serialize)]
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

/// Runs a request against an instance.
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown or non-`read` operation or unexpected
/// arguments; [`Error::Engine`] if an engine fails.
pub fn run(instance: &Instance, request: &Request) -> Result<Response, Error> {
    let Some(op) = CATALOG.iter().find(|o| o.name == request.operation) else {
        return Err(Error::Invalid(vec![format!(
            "no operation `{}`",
            request.operation
        )]));
    };
    if op.effect != Effect::Read {
        return Err(Error::Invalid(vec![format!(
            "`{}` is not a read operation",
            op.name
        )]));
    }
    if !request.args.is_empty() {
        return Err(Error::Invalid(vec![format!(
            "`{}` takes no arguments",
            op.name
        )]));
    }
    let respond = |result, violations: Vec<String>| Response {
        operation: op.name.to_string(),
        ok: violations.is_empty(),
        result,
        violations,
    };
    match op.name {
        "instance.show" => Ok(respond(json!(instance), Vec::new())),
        "policy.check" => policy_check(instance).map(|(r, v)| respond(r, v)),
        "infra.validate" => infra_validate(instance).map(|(r, v)| respond(r, v)),
        _ => unreachable!("every catalog entry is dispatched"),
    }
}

fn policy_check(instance: &Instance) -> Result<(Value, Vec<String>), Error> {
    let engine = |e: sentinel::Error| Error::Engine {
        engine: Engine::Sentinel,
        message: e.to_string(),
    };
    let policies =
        sentinel::load_dir(&instance.resolve(&instance.paths.policies)).map_err(engine)?;
    let report = sentinel::check(&policies, &instance.root, None).map_err(engine)?;
    let violations = report.violations.iter().map(ToString::to_string).collect();
    Ok((
        json!({ "ran": report.ran, "skipped": report.skipped }),
        violations,
    ))
}

fn infra_validate(instance: &Instance) -> Result<(Value, Vec<String>), Error> {
    let allowlist = instance.resolve(&instance.paths.providers);
    let dir = allowlist.parent().unwrap_or(&instance.root);
    match cobot::validate(&instance.root, dir) {
        Ok(units) => Ok((
            json!({ "units": units.iter().map(|u| u.path.as_str()).collect::<Vec<_>>() }),
            Vec::new(),
        )),
        Err(cobot::Error::Invalid(problems)) => Ok((
            json!({ "units": [] }),
            problems
                .iter()
                .map(|p| format!("{}: {}", p.path, p.message))
                .collect(),
        )),
        Err(e) => Err(Error::Engine {
            engine: Engine::Cobot,
            message: e.to_string(),
        }),
    }
}
