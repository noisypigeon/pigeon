//! The shape every capability manifest keeps; see .claude/decisions/2026-11-19-runtime-capability-interface-manifest-trait.md §5.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::{Door, Manifest, ScopeKind};
use crate::wire::Operation;

static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").expect("a valid pattern"));
static SECRET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9_]*$").expect("a valid pattern"));

/// Every problem in `manifests`, checked against each other and `catalog`, as
/// `"{name}: {message}"`, in manifest order then rule order; empty when none.
#[must_use]
pub fn validate(manifests: &[&Manifest], catalog: &[Operation]) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, m) in manifests.iter().enumerate() {
        let mut said = Vec::new();
        operations(i, manifests, catalog, &mut said);
        routes_and_storage(i, manifests, &mut said);
        the_rest(m, manifests, &mut said);
        problems.extend(said.into_iter().map(|p| format!("{}: {p}", m.name)));
    }
    problems
}

/// The manifests other than the `i`th.
fn others<'a>(i: usize, manifests: &'a [&'a Manifest]) -> impl Iterator<Item = &'a Manifest> {
    manifests
        .iter()
        .enumerate()
        .filter(move |(j, _)| *j != i)
        .map(|(_, o)| *o)
}

/// Rules 1 to 3: the version, operations in the catalog and owned once, and the scope.
fn operations(i: usize, manifests: &[&Manifest], catalog: &[Operation], said: &mut Vec<String>) {
    let m = manifests[i];
    if !VERSION.is_match(&m.version) {
        said.push("version isn't x.y.z".into());
    }
    for op in &m.operations {
        if !catalog.iter().any(|o| o.name == op) {
            said.push(format!("operation `{op}` isn't in the catalog"));
        }
        if let Some(other) = manifests[..i].iter().find(|o| o.operations.contains(op)) {
            said.push(format!("operation `{op}` is also `{}`'s", other.name));
        }
    }
    if let Some(a) = &m.scope.arg {
        for op in &m.operations {
            if let Some(o) = catalog.iter().find(|o| o.name == op)
                && !o.args.iter().any(|x| x.name == a)
            {
                said.push(format!("operation `{op}` takes no `{a}` argument"));
            }
        }
    }
    if m.scope.kind == ScopeKind::None && m.scope.arg.is_some() {
        said.push("a scope of kind none names no argument".into());
    }
    if m.scope.kind != ScopeKind::None && !m.operations.is_empty() && m.scope.arg.is_none() {
        said.push("a scoped capability names its scope argument".into());
    }
}

/// Rules 4 and 5: routes under their door and apart, storage owned once.
fn routes_and_storage(i: usize, manifests: &[&Manifest], said: &mut Vec<String>) {
    let m = manifests[i];
    for r in &m.routes {
        let p = &r.prefix;
        let door = match r.door {
            Door::App => p.starts_with("/v0/app/"),
            Door::Mcp => p.starts_with("/mcp"),
            Door::Public => true,
        };
        if !(p.starts_with('/') && p.ends_with('/') && door) {
            said.push(format!("route `{p}` isn't under its door"));
        }
        if let Some(other) = others(i, manifests).find(|o| {
            o.routes
                .iter()
                .any(|x| x.prefix.starts_with(p.as_str()) || p.starts_with(x.prefix.as_str()))
        }) {
            said.push(format!("route `{p}` overlaps `{}`'s", other.name));
        }
    }
    let s = &m.storage;
    let owned = s
        .object_keys
        .iter()
        .chain(&s.r2_prefixes)
        .chain(&s.d1_tables);
    for x in owned {
        if let Some(other) = others(i, manifests).find(|o| {
            let t = &o.storage;
            t.object_keys.contains(x) || t.r2_prefixes.contains(x) || t.d1_tables.contains(x)
        }) {
            said.push(format!("storage `{x}` is also `{}`'s", other.name));
        }
    }
    for p in &s.r2_prefixes {
        if !p.ends_with('/') {
            said.push(format!("R2 prefix `{p}` doesn't end with /"));
        }
    }
}

/// Rules 6 to 10: secrets, `built_on` and `needs`, something to grant, labels, the config.
fn the_rest(m: &Manifest, manifests: &[&Manifest], said: &mut Vec<String>) {
    for x in &m.secrets {
        if !SECRET.is_match(x) {
            said.push(format!("secret `{x}` isn't a Secrets Store name"));
        }
    }
    for x in m.built_on.iter().chain(&m.scope.needs) {
        if !manifests.iter().any(|o| o.name != m.name && &o.name == x) {
            said.push(format!("`{x}` names no capability"));
        }
    }
    let mut seen = BTreeSet::from([m.name.as_str()]);
    let mut at = m.built_on.as_deref();
    while let Some(next) = at {
        if !seen.insert(next) {
            if next == m.name {
                said.push("built_on forms a cycle".into());
            }
            break;
        }
        at = manifests
            .iter()
            .find(|o| o.name == next)
            .and_then(|o| o.built_on.as_deref());
    }
    if m.grantable && m.operations.is_empty() && m.routes.is_empty() {
        said.push("grantable with nothing to grant".into());
    }
    if m.members.label.trim().is_empty() {
        said.push("members.label is blank".into());
    }
    if m.summary.trim().is_empty() {
        said.push("summary is blank".into());
    }
    if m.config.get("type").and_then(serde_json::Value::as_str) != Some("object") {
        said.push("config isn't an object schema".into());
    }
}
