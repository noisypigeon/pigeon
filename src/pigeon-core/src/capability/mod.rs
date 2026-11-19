//! Capabilities: one manifest and one trait each, defined once and enabled per instance; see .claude/decisions/2026-11-19-runtime-capability-interface-manifest-trait.md.

use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

/// A capability's manifest, as `catalog/capabilities/<name>/capability.yaml` declares it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The capability's name, its directory under `catalog/capabilities/`.
    pub name: String,
    /// The manifest's version.
    pub version: String,
    /// One line saying what the capability is.
    pub summary: String,
    /// The capability this one is built on, if any.
    pub built_on: Option<String>,
    /// What a grant of the capability is scoped to.
    pub scope: Scope,
    /// Whether a member may be granted it today.
    pub grantable: bool,
    /// The operations it owns.
    pub operations: Vec<String>,
    /// The routes it serves.
    pub routes: Vec<Route>,
    /// The storage it owns.
    pub storage: Storage,
    /// The migrations it runs, if any.
    pub migrations: Option<String>,
    /// The secrets it is handed.
    pub secrets: Vec<String>,
    /// The JSON Schema of its per-instance config.
    pub config: serde_json::Value,
    /// The lifecycle and behaviour hooks it implements.
    pub hooks: Vec<Hook>,
    /// How members see it.
    pub members: Members,
    /// The test files that cover it.
    pub tests: Vec<String>,
}

/// What a grant is scoped to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// The kind of value the scope holds.
    pub kind: ScopeKind,
    /// The name of the scope argument, if it takes one.
    pub arg: Option<String>,
    /// Capabilities of which holding any one suffices to take this one.
    pub needs: Vec<String>,
}

/// The kind of a grant's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    /// No scope.
    None,
    /// A DNS label.
    DnsLabel,
    /// A domain's alias.
    Domain,
    /// A site's alias.
    Site,
    /// An instance's alias.
    Instance,
}

/// A route prefix a capability serves.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// The door it is served on.
    pub door: Door,
    /// The path prefix.
    pub prefix: String,
}

/// The door a route is served on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Door {
    /// The app's door.
    App,
    /// The MCP door.
    Mcp,
    /// The public door.
    Public,
}

/// The storage a capability owns.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    /// Object keys it owns.
    pub object_keys: Vec<String>,
    /// R2 prefixes it owns.
    pub r2_prefixes: Vec<String>,
    /// D1 tables it owns.
    pub d1_tables: Vec<String>,
}

/// A hook a capability implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hook {
    /// Enabled on an instance.
    Enable,
    /// Disabled on an instance.
    Disable,
    /// Upgraded between versions.
    Upgrade,
    /// A member is granted it.
    OnGrant,
    /// A member's grant is revoked.
    OnRevoke,
}

/// How members see a capability.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Members {
    /// The capability's label.
    pub label: String,
    /// The label of its scope, if it has one.
    pub scope_label: Option<String>,
    /// A line of help.
    pub help: String,
}

/// A lifecycle event of a capability on an instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    /// Enabled.
    Enable,
    /// Disabled.
    Disable,
    /// Upgraded from the named version.
    Upgrade {
        /// The version upgraded from.
        from: String,
    },
}

/// A behaviour hook around a member's grant or an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behaviour {
    /// A member is granted the capability.
    OnGrant,
    /// A member's grant is revoked.
    OnRevoke,
    /// Before the named operation.
    Before(String),
    /// After the named operation.
    After(String),
}

/// What a lifecycle hook does, each step named; dropping storage is never a step.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Storage to create.
    pub create: Vec<String>,
    /// Migrations to run.
    pub migrate: Vec<String>,
    /// Jobs to dispatch.
    pub jobs: Vec<String>,
}

/// A behaviour hook's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Let it go ahead.
    Pass,
    /// Refuse it, with the reason.
    Refuse(String),
}

/// A refusal, with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A capability: its manifest and the judgements made from it.
pub trait Capability: Sync {
    /// The capability's manifest.
    fn manifest(&self) -> &Manifest;
    /// Whether `scope` is a valid scope for the capability, by its scope kind.
    ///
    /// # Errors
    /// A [`Refusal`] naming the rule `scope` breaks.
    fn check_scope(&self, scope: Option<&str>) -> Result<(), Refusal>;
    /// Whether a grant scoped to `granted` covers a call asking for `asked`.
    fn covers(&self, granted: Option<&str>, asked: Option<&str>) -> bool;
    /// The per-instance config `raw`, checked against the manifest's config keys.
    ///
    /// # Errors
    /// A [`Refusal`] naming the first key the capability doesn't take.
    fn config(&self, raw: &serde_json::Value) -> Result<serde_json::Value, Refusal>;
    /// What the lifecycle `hook` does; by default, nothing.
    ///
    /// # Errors
    /// A [`Refusal`] when the hook can't go ahead.
    fn lifecycle(&self, hook: &Lifecycle) -> Result<Plan, Refusal> {
        let _ = hook;
        Ok(Plan::default())
    }
    /// What the behaviour `hook` answers; by default, [`Outcome::Pass`].
    fn behaviour(&self, hook: &Behaviour) -> Outcome {
        let _ = hook;
        Outcome::Pass
    }
}

/// A capability that is its manifest alone, with the default hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared(Manifest);

/// An alias: one DNS-label-shaped word, never a dotted name.
static ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$").expect("alias regex"));

impl Capability for Declared {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn check_scope(&self, scope: Option<&str>) -> Result<(), Refusal> {
        let name = &self.0.name;
        match (self.0.scope.kind, scope) {
            (ScopeKind::None, None) => Ok(()),
            (ScopeKind::None, Some(_)) => Err(Refusal(format!("`{name}` takes no scope"))),
            (ScopeKind::DnsLabel, None) => Err(Refusal(format!("`{name}` needs a label"))),
            (ScopeKind::DnsLabel, Some(l)) => {
                crate::platform::hosts::check_label(l, std::iter::empty())
                    .map_or(Ok(()), |m| Err(Refusal(m)))
            }
            (_, Some(s)) if ALIAS.is_match(s) => Ok(()),
            (_, _) => Err(Refusal(format!("{name}'s scope is an alias of [a-z0-9-]"))),
        }
    }

    fn covers(&self, granted: Option<&str>, asked: Option<&str>) -> bool {
        self.0.scope.kind == ScopeKind::None || (granted.is_some() && granted == asked)
    }

    fn config(&self, raw: &serde_json::Value) -> Result<serde_json::Value, Refusal> {
        let known = self
            .0
            .config
            .get("properties")
            .and_then(serde_json::Value::as_object);
        let Some(obj) = raw.as_object() else {
            return Err(Refusal(format!("{}'s config is an object", self.0.name)));
        };
        match obj
            .keys()
            .find(|k| !known.is_some_and(|p| p.contains_key(*k)))
        {
            Some(k) => Err(Refusal(format!("{}'s config takes no `{k}`", self.0.name))),
            None => Ok(raw.clone()),
        }
    }
}

/// Every manifest, by directory name, embedded.
const SOURCES: [(&str, &str); 6] = [
    (
        "blog",
        include_str!("../../../../catalog/capabilities/blog/capability.yaml"),
    ),
    (
        "domain",
        include_str!("../../../../catalog/capabilities/domain/capability.yaml"),
    ),
    (
        "icbc",
        include_str!("../../../../catalog/capabilities/icbc/capability.yaml"),
    ),
    (
        "instances",
        include_str!("../../../../catalog/capabilities/instances/capability.yaml"),
    ),
    (
        "sites",
        include_str!("../../../../catalog/capabilities/sites/capability.yaml"),
    ),
    (
        "subdomain",
        include_str!("../../../../catalog/capabilities/subdomain/capability.yaml"),
    ),
];

/// The registry: every embedded manifest, parsed.
static REGISTRY: LazyLock<Vec<Declared>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(_, y)| Declared(serde_saphyr::from_str::<Manifest>(y).expect("capability manifest")))
        .collect()
});

/// Every capability, in registry order.
#[must_use]
pub fn registry() -> &'static [Declared] {
    &REGISTRY
}

/// The capability named `name`, if any.
#[must_use]
pub fn get(name: &str) -> Option<&'static Declared> {
    REGISTRY.iter().find(|c| c.0.name == name)
}

/// Every capability's name, in registry order.
#[must_use]
pub fn names() -> Vec<&'static str> {
    REGISTRY.iter().map(|c| c.0.name.as_str()).collect()
}

/// The names of the capabilities a member may be granted, in registry order.
#[must_use]
pub fn grantable() -> Vec<&'static str> {
    REGISTRY
        .iter()
        .filter(|c| c.0.grantable)
        .map(|c| c.0.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests;

/// The capability that owns operation `op`, and the argument naming its scope, if any.
#[must_use]
pub fn owner_of(op: &str) -> Option<(&'static str, Option<&'static str>)> {
    registry().iter().find_map(|c| {
        let m = c.manifest();
        m.operations
            .iter()
            .any(|o| o == op)
            .then_some((m.name.as_str(), m.scope.arg.as_deref()))
    })
}
