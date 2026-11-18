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
        "architecture.describe" => {
            let (r, v) = architecture_describe(instance, &var)?;
            let format = request.args.get("format").and_then(Value::as_str);
            let r = crate::architecture::render::formatted(r, format)
                .map_err(|e| Error::Invalid(vec![e]))?;
            Ok(respond(r, v))
        }
        "policy.check" => policy_check(instance, &request.args, &var).map(|(r, v)| respond(r, v)),
        "change.scope" => change_scope(instance, &request.args).map(|r| respond(r, Vec::new())),
        "sites.publish" => sites_publish(instance, &request.args, &var).map(|(r, v)| respond(r, v)),
        "sites.build" => sites_build(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        "infra.validate" => infra_validate(instance).map(|(r, v)| respond(r, v)),
        "infra.plan" => infra_plan(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        "infra.apply" => infra_apply(instance, &request.args, &var).map(|r| respond(r, Vec::new())),
        _ if op.control_plane() => Err(Error::Invalid(vec![format!(
            "`{}` runs in the control plane, which holds every change",
            op.name
        )])),
        _ => unreachable!("every catalog entry is dispatched"),
    }
}

/// The graph and its layout, with any problem found on the way as a violation.
fn architecture_describe(
    instance: &Instance,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<(Value, Vec<String>), Error> {
    let (graph, problems) = crate::architecture::describe_with(instance, var)?;
    let layout = crate::architecture::layout(&graph);
    let network = crate::architecture::network(&instance.root, instance.platform_root());
    let mut result = json!({
        "nodes": graph.nodes,
        "edges": graph.edges,
        "groups": graph.groups,
        "layout": layout,
        "network": network,
    });
    let zones = zones(instance, &result);
    let roots: Vec<String> = [&instance.root, &instance.platform_root().to_path_buf()]
        .iter()
        .map(|r| format!("{}/", r.display()))
        .collect();
    scrub(&mut result, &zones, &roots);
    Ok((result, problems))
}

/// Every zone but `pigeon.dev`'s the graph would name: each site's host, and each workload's
/// directory, longest first.
fn zones(instance: &Instance, result: &Value) -> Vec<String> {
    let ours = |h: &str| h == "pigeon.dev" || h.ends_with(".pigeon.dev");
    let mut out: Vec<String> = result["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["kind"] == "site")
        .filter_map(|n| n["label"].as_str())
        .filter(|h| h.contains('.') && !ours(h))
        .map(|h| h.trim_start_matches("www.").to_string())
        .collect();
    for root in [&instance.root, &instance.platform_root().to_path_buf()] {
        for e in std::fs::read_dir(root.join("workloads"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.contains('.') && !ours(&name) {
                out.push(name);
            }
        }
    }
    out.sort();
    out.dedup();
    out.sort_by_key(|z| std::cmp::Reverse(z.len()));
    out
}

/// Replaces each of `zones` in every string of `v` with `zone-<n>`, and drops `roots`, so the
/// graph names no zone but `pigeon.dev` and no checkout's path; see
/// `.claude/decisions/2026-11-15-runtime-architecture-coverage-agents-deploys.md` §2.
fn scrub(v: &mut Value, zones: &[String], roots: &[String]) {
    match v {
        Value::String(s) => {
            for r in roots {
                *s = s.replace(r.as_str(), "");
            }
            for (i, z) in zones.iter().enumerate() {
                if s.contains(z.as_str()) {
                    *s = s.replace(z.as_str(), &format!("zone-{}", i + 1));
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| scrub(x, zones, roots)),
        Value::Object(o) => o.values_mut().for_each(|x| scrub(x, zones, roots)),
        _ => {}
    }
}

/// The pull request `args` describe, as `pigeon-eviction`'s `PR_*` variables, if they name any
/// of its fields but the scope.
fn pull_request(args: &Map<String, Value>) -> Option<Vec<(&'static str, String)>> {
    let field = |k: &str| match args.get(k)? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    };
    let fields = [
        ("PR_TITLE", "title"),
        ("PR_BODY", "body"),
        ("PR_NUMBER", "number"),
        ("PR_FILES", "files"),
        ("PR_BRANCH", "branch"),
        ("PR_SCOPE", "scope"),
    ];
    let named = fields
        .iter()
        .any(|(_, k)| *k != "scope" && field(k).is_some());
    named.then(|| {
        fields
            .iter()
            .filter_map(|(var, k)| Some((*var, field(k)?)))
            .collect()
    })
}

fn policy_check(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<(Value, Vec<String>), Error> {
    let Some(dir) = instance.policies_dir() else {
        return Ok((json!({ "ran": 0, "skipped": 0 }), Vec::new()));
    };
    let binary = crate::eviction::locate(var);
    let pr = pull_request(args);
    let report =
        crate::eviction::check(&binary, &instance.root, &dir, pr.as_deref()).map_err(|e| {
            Error::Engine {
                engine: Engine::Eviction,
                message: e.to_string(),
            }
        })?;
    Ok((
        json!({ "ran": report.ran, "skipped": report.skipped }),
        report.violations,
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
    let jobs = crate::checks::scope::jobs(instance.platform_root(), &files).map_err(|e| {
        Error::Engine {
            engine: Engine::Infra,
            message: e.to_string(),
        }
    })?;
    let deploys = crate::checks::scope::plan(instance.platform_root(), &files).map_err(|e| {
        Error::Engine {
            engine: Engine::Infra,
            message: e.to_string(),
        }
    })?;
    Ok(json!({ "jobs": jobs, "deploys": deploys }))
}

/// Publishes the named sites, or every site, with the run's Cloudflare token: built with
/// `architecture.describe`'s result as data, synced to each bucket, then checked live. A failed
/// live check is a violation.
fn sites_publish(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<(Value, Vec<String>), Error> {
    let token = var(crate::workers::cloudflare::TOKEN_VAR).ok_or_else(|| {
        Error::Invalid(vec![format!(
            "`sites.publish` needs {} with R2 write on the account",
            crate::workers::cloudflare::TOKEN_VAR
        )])
    })?;
    let wanted = strings(args, "sites");
    let ctx = crate::infra::Context {
        root: instance.root.clone(),
        providers: instance.providers_dir(),
        state: std::env::temp_dir(),
        state_passphrase: String::new(),
        secrets_passphrase: None,
        workload_secrets_passphrase: None,
        today: crate::infra::generations::today(),
        credentials: var(crate::infra::CREDENTIALS_VAR),
        cloudflare_token: Some(token),
        cloudflare_api: None,
        platform: instance.platform.clone(),
        adopt: false,
        provider_config: None,
        provider_token: false,
    };
    let (mut architecture, _) = architecture_describe(instance, var)?;
    if let Some(sha) = args.get("deployed").and_then(Value::as_str) {
        architecture["deployed"] = json!(sha);
    }
    let data = std::collections::BTreeMap::from([("architecture".to_string(), architecture)]);
    let published = crate::infra::publish(&ctx, &wanted, 5, &data).map_err(|e| infra_failed(&e))?;
    let violations = published
        .iter()
        .flat_map(|p| {
            p.checks
                .iter()
                .filter(|c| !c.passed)
                .map(move |c| format!("{}: {} failed", p.unit, c.what))
        })
        .collect();
    Ok((
        json!({
            "sites": published.iter().map(|p| json!({
                "unit": p.unit,
                "put": p.synced.put.len(),
                "unchanged": p.synced.unchanged,
                "deleted": p.synced.deleted.len(),
            })).collect::<Vec<_>>(),
        }),
        violations,
    ))
}

fn strings(args: &Map<String, Value>, k: &str) -> Vec<String> {
    args.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Builds the named sites, or every site, with `architecture.describe`'s result as their
/// `data/architecture.json`, per the architecture-site ADR §3.
fn sites_build(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<Value, Error> {
    let infra_error = |e: crate::infra::Error| Error::Engine {
        engine: Engine::Infra,
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
    let sites = crate::sites::find_in(&instance.root, instance.platform_root(), &wanted)
        .map_err(infra_error)?;
    if sites.is_empty() {
        return Ok(json!({ "sites": [] }));
    }
    let (architecture, _) = architecture_describe(instance, var)?;
    let data = std::collections::BTreeMap::from([("architecture".to_string(), architecture)]);
    let built = crate::sites::build_in(&instance.root, instance.platform_root(), &sites, &data)
        .map_err(infra_error)?;
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
    match crate::infra::validate_in(&instance.root, instance.platform_root(), &dir) {
        Ok(units) => Ok((
            json!({ "units": units.iter().map(|u| u.path.as_str()).collect::<Vec<_>>() }),
            Vec::new(),
        )),
        Err(crate::infra::Error::Invalid(problems)) => Ok((
            json!({ "units": [] }),
            problems
                .iter()
                .map(|p| format!("{}: {}", p.path, p.message))
                .collect(),
        )),
        Err(e) => Err(Error::Engine {
            engine: Engine::Infra,
            message: e.to_string(),
        }),
    }
}

fn text<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, Error> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid(vec![format!("argument `{name}` must be a string")]))
}

fn infra_failed(e: &crate::infra::Error) -> Error {
    Error::Engine {
        engine: Engine::Infra,
        message: e.to_string(),
    }
}

fn session(
    instance: &Instance,
    args: &Map<String, Value>,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<crate::infra::session::Session, Error> {
    let providers = instance.providers_dir();
    crate::infra::session::open(&instance.root, &providers, text(args, "state")?, var)
        .map(|mut s| {
            s.ctx.platform.clone_from(&instance.platform);
            s
        })
        .map_err(|e| infra_failed(&e))
}

fn changes(changes: &[crate::infra::Change]) -> Value {
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
    let selected =
        crate::infra::select_in(&s.ctx.root, s.ctx.platform_root(), &s.ctx.providers, &units)
            .map_err(|e| infra_failed(&e))?;
    let (stored, _) = crate::infra::plan_stored(&s.ctx, &selected, crate::infra::session::now())
        .map_err(|e| infra_failed(&e))?;
    s.push().map_err(|e| infra_failed(&e))?;
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
    let applied = s.apply_stored(plan, crate::infra::session::now());
    s.push().map_err(|e| infra_failed(&e))?;
    let applied = applied.map_err(|e| infra_failed(&e))?;
    Ok(json!({
        "plan": plan,
        "units": applied.iter().map(|u| json!({
            "unit": u.unit,
            "changes": changes(&u.changes),
            "note": u.note,
        })).collect::<Vec<_>>(),
    }))
}
