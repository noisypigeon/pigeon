//! Running the catalog's operations against an instance: the `engines` build.

use serde_json::{Map, Value, json};

use crate::wire::{Engine, Request, Response, validate};
use crate::{Error, Instance};

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
    let op = validate(request)?;
    let respond = |result, violations: Vec<String>| Response {
        operation: op.name.to_string(),
        ok: violations.is_empty(),
        result,
        violations,
    };
    match op.name {
        "instance.show" => Ok(respond(json!(instance), Vec::new())),
        "architecture.describe" => architecture_describe(instance).map(|(r, v)| respond(r, v)),
        "policy.check" => policy_check(instance, &request.args).map(|(r, v)| respond(r, v)),
        "change.scope" => change_scope(instance, &request.args).map(|r| respond(r, Vec::new())),
        "sites.build" => sites_build(instance, &request.args).map(|r| respond(r, Vec::new())),
        "infra.validate" => infra_validate(instance).map(|(r, v)| respond(r, v)),
        "infra.plan" => infra_plan(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        "infra.apply" => infra_apply(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        _ => unreachable!("every catalog entry is dispatched"),
    }
}

/// The graph and its layout, with any problem found on the way as a violation.
fn architecture_describe(instance: &Instance) -> Result<(Value, Vec<String>), Error> {
    let (graph, problems) = crate::architecture::describe(instance)?;
    let layout = crate::architecture::layout(&graph);
    Ok((
        json!({
            "nodes": graph.nodes,
            "edges": graph.edges,
            "groups": graph.groups,
            "layout": layout,
        }),
        problems,
    ))
}

/// The pull request `args` describe, if they name any of its fields.
fn pull_request(args: &Map<String, Value>) -> Option<sentinel::PullRequest> {
    let field = |k: &str| match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    };
    let pr = sentinel::PullRequest {
        title: field("title"),
        body: field("body"),
        number: field("number"),
        files: field("files"),
        branch: field("branch"),
        scope: field("scope"),
    };
    (pr.title.is_some()
        || pr.body.is_some()
        || pr.number.is_some()
        || pr.files.is_some()
        || pr.branch.is_some())
    .then_some(pr)
}

fn policy_check(
    instance: &Instance,
    args: &Map<String, Value>,
) -> Result<(Value, Vec<String>), Error> {
    let engine = |e: sentinel::Error| Error::Engine {
        engine: Engine::Sentinel,
        message: e.to_string(),
    };
    let Some(dir) = instance.policies_dir() else {
        return Ok((json!({ "ran": 0, "skipped": 0 }), Vec::new()));
    };
    let policies = sentinel::load_dir(&dir).map_err(engine)?;
    let pr = pull_request(args);
    let report = sentinel::check(&policies, &instance.root, pr.as_ref()).map_err(engine)?;
    let violations = report.violations.iter().map(ToString::to_string).collect();
    Ok((
        json!({ "ran": report.ran, "skipped": report.skipped }),
        violations,
    ))
}

/// The checks a change touching `files` needs, from the platform's workspace and release
/// configuration; runs nothing, per the lifecycle ADR §4.
fn change_scope(instance: &Instance, args: &Map<String, Value>) -> Result<Value, Error> {
    let files: Vec<String> = args
        .get("files")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let jobs = cobot::scope::jobs(instance.platform_root(), &files).map_err(|e| Error::Engine {
        engine: Engine::Cobot,
        message: e.to_string(),
    })?;
    Ok(json!({ "jobs": jobs }))
}

/// Builds the named sites, or every site, with `architecture.describe`'s result as their
/// `data/architecture.json`, per the architecture-site ADR §3.
fn sites_build(instance: &Instance, args: &Map<String, Value>) -> Result<Value, Error> {
    let cobot = |e: cobot::Error| Error::Engine {
        engine: Engine::Cobot,
        message: e.to_string(),
    };
    let wanted: Vec<String> = args
        .get("sites")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let sites =
        cobot::sites::find_in(&instance.root, instance.platform_root(), &wanted).map_err(cobot)?;
    if sites.is_empty() {
        return Ok(json!({ "sites": [] }));
    }
    let (architecture, _) = architecture_describe(instance)?;
    let data = std::collections::BTreeMap::from([("architecture".to_string(), architecture)]);
    let built = cobot::sites::build_in(&instance.root, instance.platform_root(), &sites, &data)
        .map_err(cobot)?;
    Ok(json!({
        "sites": built
            .iter()
            .map(|b| json!({
                "unit": b.site.unit,
                "template": b.site.tag(),
                "features": b.features,
            }))
            .collect::<Vec<_>>(),
    }))
}

fn infra_validate(instance: &Instance) -> Result<(Value, Vec<String>), Error> {
    let dir = instance.providers_dir();
    match cobot::validate_in(&instance.root, instance.platform_root(), &dir) {
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
    let providers = instance.providers_dir();
    cobot::session::open(&instance.root, &providers, text(args, "state")?, var)
        .map(|mut s| {
            s.ctx.platform.clone_from(&instance.platform);
            s
        })
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
    let mut s = session(instance, args, var)?;
    let selected = cobot::select_in(&s.ctx.root, s.ctx.platform_root(), &s.ctx.providers, &units)
        .map_err(|e| cobot_failed(&e))?;
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

/// Applies a stored plan exactly; each unit's changes. R2 state is written back after each unit,
/// and again even when the apply fails.
fn infra_apply(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Value, Error> {
    let plan = text(args, "plan")?;
    let mut s = session(instance, args, var)?;
    let applied = s.apply_stored(plan, cobot::session::now());
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
