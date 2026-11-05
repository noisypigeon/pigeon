//! Derives the graph from the tree, per §2's table: only what the engines read appears.

use std::collections::BTreeSet;
use std::path::Path;

use sentinel::policy::{NamedTarget, Target};

use super::{Edge, Graph, Group, Lane, Node};
use crate::wire::{Engine, catalog};
use crate::{Error, Instance, engines};

const REPOSITORY: &str = "repository";
const PULL_REQUEST: &str = "pull-request";

fn node(id: &str, kind: &str, lane: Lane, label: &str, source: &str) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        lane,
        label: label.into(),
        source: source.into(),
    }
}

fn edge(from: &str, to: &str, carries: &str, source: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        carries: carries.into(),
        source: source.into(),
    }
}

/// The unit holding `path`, or the repository.
fn owner(units: &[String], path: &Path) -> String {
    let path = path.to_string_lossy();
    units
        .iter()
        .filter(|u| path == u.as_str() || path.starts_with(&format!("{u}/")))
        .max_by_key(|u| u.len())
        .map_or_else(|| REPOSITORY.to_string(), |u| format!("unit:{u}"))
}

/// The instance's architecture, derived from its tree; each problem found on the way is
/// returned beside it.
///
/// # Errors
///
/// [`Error::Engine`] if sentinel cannot load the policies.
pub fn describe(instance: &Instance) -> Result<(Graph, Vec<String>), Error> {
    let mut g = Graph::default();
    let mut problems = Vec::new();
    engines_and_operations(&mut g);
    let units = units(instance, &mut g, &mut problems);
    sites(
        &instance.root,
        instance.platform_root(),
        &mut g,
        &mut problems,
    );
    policies(instance, &units, &mut g)?;
    g.normalize();
    Ok((g, problems))
}

/// Every engine, and each operation it runs.
fn engines_and_operations(g: &mut Graph) {
    g.nodes.push(node(
        "engine:backplane",
        "engine",
        Lane::Run,
        "backplane",
        "backplane::engines()",
    ));
    for e in engines() {
        let name = e.engine.name();
        g.nodes.push(node(
            &format!("engine:{name}"),
            "engine",
            Lane::Run,
            name,
            "backplane::engines()",
        ));
    }
    for op in catalog() {
        let id = format!("operation:{}", op.name);
        g.nodes.push(node(
            &id,
            "operation",
            Lane::Operations,
            op.name,
            "backplane::catalog()",
        ));
        g.edges.push(edge(
            &id,
            &format!("engine:{}", op.engine.name()),
            "runs",
            "backplane::catalog()",
        ));
    }
}

/// Every unit, its providers, the capabilities it requests, and a group per local unit; the
/// units' paths.
fn units(instance: &Instance, g: &mut Graph, problems: &mut Vec<String>) -> Vec<String> {
    let root = &instance.root;
    g.nodes
        .push(node(REPOSITORY, "source", Lane::Source, "repository", "."));
    // Every problem `infra.validate` reports, discovery's included; the graph keeps the
    // units discovery could read.
    match cobot::validate_in(root, instance.platform_root(), &instance.providers_dir()) {
        Ok(_) => {}
        Err(cobot::Error::Invalid(found)) => {
            problems.extend(found.iter().map(|p| format!("{}: {}", p.path, p.message)));
        }
        Err(e) => problems.push(format!("{}: {e}", Engine::Cobot.name())),
    }
    let (units, _) = cobot::unit::discover_in(root, instance.platform_root());
    let paths: Vec<String> = units.iter().map(|u| u.path.clone()).collect();
    for u in &units {
        let id = format!("unit:{}", u.path);
        let file = if u.is_config() {
            cobot::config::CONFIG
        } else {
            "unit.yaml"
        };
        g.nodes.push(node(
            &id,
            "unit",
            Lane::Units,
            &u.path,
            &format!("{}/{file}", u.path),
        ));
        let Some(c) = &u.config else { continue };
        let x = &c.schema["x-cobot"];
        let schema = c.home.strip_prefix(root).map_or_else(
            |_| c.home.display().to_string(),
            |p| p.display().to_string(),
        );
        for provider in x["providers"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            g.nodes.push(node(
                &format!("provider:{provider}"),
                "provider",
                Lane::Providers,
                provider,
                ".config/providers.yaml",
            ));
            g.edges.push(edge(
                &id,
                &format!("provider:{provider}"),
                "apply",
                &format!("{schema}/schema.json x-cobot.providers"),
            ));
        }
        for (name, request) in x["credentials"].as_object().into_iter().flatten() {
            let Some(provider) = request["provider"].as_str() else {
                continue;
            };
            let source = format!("{schema}/schema.json x-cobot.credentials.{name}");
            for cap in request["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
            {
                let cid = format!("capability:{provider}.{cap}");
                g.nodes.push(node(
                    &cid,
                    "capability",
                    Lane::Providers,
                    &format!("{provider} {cap}"),
                    &source,
                ));
                g.edges.push(edge(&id, &cid, "credential", &source));
            }
        }
        if x["local"].as_bool() == Some(true) {
            g.groups.push(Group {
                id: format!("local:{}", u.path),
                kind: "local".into(),
                label: "local unit".into(),
                members: vec![id.clone()],
                parent: None,
            });
        }
    }

    paths
}

/// Every static site's bucket, named by its unit, and the hosts it serves.
fn sites(root: &Path, platform: &Path, g: &mut Graph, problems: &mut Vec<String>) {
    match cobot::sites::find_in(root, platform, &[]) {
        Ok(sites) => {
            for s in sites {
                let store = format!("store:{}", s.unit);
                g.nodes.push(node(
                    &store,
                    "store",
                    Lane::Data,
                    &format!("{} bucket", s.workload),
                    &format!("{}/config.yaml static_site", s.unit),
                ));
                for host in &s.hosts {
                    let site = format!("site:{host}");
                    g.nodes.push(node(
                        &site,
                        "site",
                        Lane::Sites,
                        host,
                        &format!("{}/config.yaml static_site", s.unit),
                    ));
                    g.edges
                        .push(edge(&store, &site, "artifact", "static_site custom domain"));
                }
            }
        }
        Err(cobot::Error::Invalid(_)) => {}
        Err(e) => problems.push(format!("{}: {e}", Engine::Cobot.name())),
    }
}

/// Every policy, and what it governs.
fn policies(instance: &Instance, units: &[String], g: &mut Graph) -> Result<(), Error> {
    let (Some(rel), Some(dir)) = (&instance.paths.policies, instance.policies_dir()) else {
        return Ok(());
    };
    let policies_dir = rel.display().to_string();
    let unreadable = |e: std::io::Error| Error::Engine {
        engine: Engine::Sentinel,
        message: format!("{policies_dir}: {e}"),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(unreadable(e)),
    };
    let mut any = false;
    for entry in entries {
        if entry
            .map_err(unreadable)?
            .path()
            .extension()
            .is_some_and(|x| x == "yaml")
        {
            any = true;
        }
    }
    if !any {
        return Ok(());
    }
    let policies = sentinel::load_dir(&dir).map_err(|e| Error::Engine {
        engine: Engine::Sentinel,
        message: e.to_string(),
    })?;
    for p in &policies {
        let id = format!("policy:{}", p.name);
        let source = format!("{policies_dir}/{}.yaml", p.name);
        g.nodes
            .push(node(&id, "policy", Lane::Policy, &p.name, &source));
        let targets: BTreeSet<String> = match &p.policy.target {
            Target::Named(NamedTarget::PullRequest | NamedTarget::Change) => {
                g.nodes.push(node(
                    PULL_REQUEST,
                    "source",
                    Lane::Source,
                    "pull request",
                    &source,
                ));
                BTreeSet::from([PULL_REQUEST.to_string()])
            }
            Target::Named(NamedTarget::Tree) => BTreeSet::from([REPOSITORY.to_string()]),
            Target::File(f) => BTreeSet::from([owner(units, &f.file)]),
            Target::Files(f) => f.files.iter().map(|p| owner(units, p)).collect(),
            Target::Dir(d) => BTreeSet::from([owner(units, &d.dir)]),
        };
        for t in targets {
            g.edges
                .push(edge(&id, &t, "governs", &format!("{source} target")));
        }
    }

    Ok(())
}
