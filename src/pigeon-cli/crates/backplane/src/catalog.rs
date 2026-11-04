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
    Operation {
        name: "infra.plan",
        engine: Engine::Cobot,
        effect: Effect::Plan,
        summary: "Plan units and store the plan for 24 hours (args: state, units?)",
    },
    Operation {
        name: "infra.apply",
        engine: Engine::Cobot,
        effect: Effect::Apply,
        summary: "Apply a stored, unexpired plan exactly (args: state, plan)",
    },
];

/// The arguments an operation takes: each name, and whether it is required.
fn arguments(name: &str) -> &'static [(&'static str, bool)] {
    match name {
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

/// Runs a request against an instance, reading passphrases and credentials from the
/// environment.
///
/// # Errors
///
/// As [`run_with`].
pub fn run(instance: &Instance, request: &Request) -> Result<Response, Error> {
    run_with(instance, request, |k| std::env::var(k).ok())
}

/// Runs a request against an instance, reading passphrases and credentials through `var`.
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown operation or bad arguments; [`Error::Engine`] if an
/// engine fails.
pub fn run_with(
    instance: &Instance,
    request: &Request,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Response, Error> {
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
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
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
        "infra.plan" => infra_plan(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        "infra.apply" => infra_apply(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
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

fn text<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, Error> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid(vec![format!("argument `{name}` must be a string")]))
}

fn cobot_failed(e: &cobot::Error) -> Error {
    Error::Engine {
        engine: Engine::Cobot,
        message: e.to_string(),
    }
}

fn session(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<cobot::session::Session, Error> {
    let allowlist = instance.resolve(&instance.paths.providers);
    let providers = allowlist.parent().unwrap_or(&instance.root);
    cobot::session::open(&instance.root, providers, text(args, "state")?, var)
        .map_err(|e| cobot_failed(&e))
}

fn changes(changes: &[cobot::Change]) -> Value {
    Value::Array(
        changes
            .iter()
            .map(|c| json!({ "address": c.address, "action": c.action.word() }))
            .collect(),
    )
}

/// Plans and stores; the plan's ID, expiry, and each unit's changes.
fn infra_plan(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Value, Error> {
    let units: Vec<String> = match args.get("units") {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(|u| u.as_str().map(str::to_string)).collect())
            .ok_or_else(|| {
                Error::Invalid(vec!["argument `units` must be a list of strings".into()])
            })?,
    };
    let s = session(instance, args, var)?;
    let selected =
        cobot::select(&s.ctx.root, &s.ctx.providers, &units).map_err(|e| cobot_failed(&e))?;
    let (stored, _) = cobot::plan_stored(&s.ctx, &selected, cobot::session::now())
        .map_err(|e| cobot_failed(&e))?;
    s.push().map_err(|e| cobot_failed(&e))?;
    Ok(json!({
        "plan": stored.id,
        "expires": stored.expires,
        "units": stored.units.iter().map(|u| json!({
            "unit": u.unit,
            "changes": changes(&u.changes),
        })).collect::<Vec<_>>(),
    }))
}

/// Applies a stored plan exactly; each unit's changes. R2 state is written back even when the
/// apply fails.
fn infra_apply(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Value, Error> {
    let plan = text(args, "plan")?;
    let s = session(instance, args, var)?;
    let applied = cobot::apply_stored(&s.ctx, plan, cobot::session::now());
    s.push().map_err(|e| cobot_failed(&e))?;
    let applied = applied.map_err(|e| cobot_failed(&e))?;
    Ok(json!({
        "plan": plan,
        "units": applied.iter().map(|u| json!({
            "unit": u.unit,
            "changes": changes(&u.changes),
            "note": u.note,
        })).collect::<Vec<_>>(),
    }))
}
