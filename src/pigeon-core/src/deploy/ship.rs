//! A Worker deploy's judgements: whether it must be whole, what a deployment holds, and whether a
//! version answers healthy; see `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`
//! and `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 2's spike.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// How long a new version has to answer its health check: longer than [`CODE_UPDATE_DELAY`].
pub const HEALTH_WAIT: Duration = Duration::from_secs(120);

/// The longest a busy Durable Object keeps old code after a deployment, in seconds.
pub const CODE_UPDATE_DELAY: u32 = 60;

/// The header that sends a request to a version in the current deployment.
pub const OVERRIDE_HEADER: &str = "Cloudflare-Workers-Version-Overrides";

/// Why a deploy of the Worker in `dir` must be a whole `wrangler deploy`: no earlier version, its
/// changed files unknown, a file of its own changed (its spec, a migration, a class), or its
/// images rebuilt; none when version blue-green applies.
#[must_use]
pub fn whole_deploy(
    dir: &str,
    files: Option<&[String]>,
    images_built: bool,
    from: Option<&str>,
) -> Option<String> {
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    if from.is_none() {
        return Some("no version serves yet".into());
    }
    if images_built {
        return Some("its container images were rebuilt".into());
    }
    let Some(files) = files else {
        return Some("every input is deployed again".into());
    };
    files
        .iter()
        .find(|f| f.starts_with(&prefix))
        .map(|f| format!("{f} changed, which may change an object class or a container"))
}

/// The version ID `wrangler versions upload` printed.
#[must_use]
pub fn uploaded_version(out: &str) -> Option<String> {
    out.lines()
        .filter(|l| l.contains("Version ID"))
        .find_map(|l| {
            l.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
                .find(|w| w.len() == 36 && w.matches('-').count() == 4)
        })
        .map(str::to_string)
}

/// Whether a `/healthz/smoke` answer is `version`'s, with every probe passed.
#[must_use]
pub fn smoke_passed(answer: &Value, version: &str) -> bool {
    answer["ok"] == true
        && answer["version"].as_str() == Some(version)
        && answer["probes"]
            .as_object()
            .is_some_and(|m| !m.is_empty() && m.values().all(|v| v == true))
}

/// A deployment of `versions`, each at its percentage, under a Durable Object code update
/// strategy: deferred by [`CODE_UPDATE_DELAY`], or immediate.
#[must_use]
pub fn deployment(versions: &[(&str, u8)], immediate: bool) -> Value {
    let strategy = if immediate {
        json!({ "mode": "immediate" })
    } else {
        json!({ "mode": "deferred", "max_delay": CODE_UPDATE_DELAY })
    };
    json!({
        "strategy": "percentage",
        "versions": versions
            .iter()
            .map(|(id, p)| json!({ "version_id": id, "percentage": p }))
            .collect::<Vec<_>>(),
        "code_update_strategy": strategy,
    })
}

/// Each unit's outputs, by name.
pub type Outputs = BTreeMap<String, Map<String, Value>>;

/// The images a deploy references: each container's, keyed as `cobot::workers::image_key`, and the
/// commit they were built from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Images {
    /// The commit the images were built from.
    pub source: String,
    /// Each container's registry reference.
    pub images: BTreeMap<String, String>,
}

/// Every string in `outputs` worth scrubbing from a log: IDs and names of eight or more
/// characters.
#[must_use]
pub fn secrets_of(outputs: &Outputs, images: &Images) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) if s.len() >= 8 => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for o in outputs.values() {
        o.values().for_each(|v| walk(v, &mut out));
    }
    out.extend(images.images.values().cloned());
    out
}

/// The version serving all of a deployment's traffic, from the API's deployments answer.
#[must_use]
pub fn serving(deployments: &Value) -> Option<String> {
    let list = deployments["deployments"]
        .as_array()
        .or_else(|| deployments.as_array())?;
    let latest = list.first()?;
    latest["versions"]
        .as_array()?
        .iter()
        .max_by(|a, b| {
            let p = |v: &Value| v["percentage"].as_f64().unwrap_or(0.0);
            p(a).total_cmp(&p(b))
        })
        .and_then(|v| v["version_id"].as_str())
        .map(str::to_string)
}

/// Whether a `/healthz` answer is `version`'s and healthy.
#[must_use]
pub fn healthy(answer: &Value, version: &str) -> bool {
    answer["ok"] == true && answer["version"].as_str() == Some(version)
}

/// What the shell's `ship` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shipped {
    /// The Worker.
    pub name: String,
    /// The version it served before.
    pub from: Option<String>,
    /// The version it serves now.
    pub to: String,
    /// What was done, step by step.
    pub steps: Vec<String>,
}
