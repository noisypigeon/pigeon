//! Worker deploys from the pipeline, with no Docker and no state passphrase: outputs from a
//! snapshot, images by registry reference, then health and rollback; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`.

use std::collections::BTreeMap;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Error;
use crate::cloudflare::Api;
use crate::workers::{self, Found, Spec, Tools};

pub use crate::ship_wire::{
    ACCESS_ID_VAR, ACCESS_SECRET_VAR, HOST_VAR, IMAGES_VAR, OUTPUTS_VAR, TOKEN_VAR,
};

/// How long a new version has to answer its health check.
pub const HEALTH_WAIT: Duration = Duration::from_secs(120);

/// Each unit's outputs, by name.
pub type Outputs = BTreeMap<String, Map<String, Value>>;

/// The outputs of `outputs` that `specs` reference, and nothing else: what
/// [`OUTPUTS_VAR`] holds.
#[must_use]
pub fn snapshot(specs: &[Spec], outputs: &Outputs) -> Outputs {
    let mut out = Outputs::new();
    for r in specs.iter().flat_map(workers::references) {
        let Some(x) = workers::reference(&r) else {
            continue;
        };
        if let Some(v) = outputs.get(&x.unit).and_then(|o| o.get(&x.output)) {
            out.entry(x.unit).or_default().insert(x.output, v.clone());
        }
    }
    out
}

/// The images a deploy references: each container's, keyed as [`workers::image_key`], and the
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

/// What [`ship`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shipped {
    /// The Worker.
    pub name: String,
    /// The version it served before.
    pub from: Option<String>,
    /// The version it serves now.
    pub to: String,
}

/// Where a deploy runs and how it checks itself.
pub struct Target<'a> {
    /// The account's API, with the deploy token.
    pub api: &'a Api,
    /// The account.
    pub account: &'a str,
    /// The host `/healthz` is asked on.
    pub host: &'a str,
    /// The Access service token's client ID and secret.
    pub access: (&'a str, &'a str),
}

fn tool(message: impl Into<String>) -> Error {
    Error::Tool {
        command: "workers ship".into(),
        message: message.into(),
    }
}

fn deployments(t: &Target, name: &str) -> Result<Value, Error> {
    t.api
        .get(&format!(
            "/accounts/{}/workers/scripts/{name}/deployments",
            t.account
        ))
        .map_err(tool)
}

/// `GET https://<host>/healthz` with the Access service token; its JSON.
fn health(t: &Target) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut r = agent
        .get(&format!("https://{}/healthz", t.host))
        .header("CF-Access-Client-Id", t.access.0)
        .header("CF-Access-Client-Secret", t.access.1)
        .call()
        .map_err(|_| "the health check's request failed".to_string())?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    serde_json::from_str(&body).map_err(|_| format!("/healthz answered {status}, not JSON"))
}

/// Deploys `found` from the checkout at `root`: renders it with `outputs` and `images`, applies
/// its migrations, deploys it, and waits for the new version's health; on a failed check it
/// rolls back to the version that served before.
///
/// # Errors
///
/// [`Error::Invalid`] for a reference or image the snapshots lack; [`Error::Tool`] for a failed
/// step, after any rollback, naming what was done.
pub fn ship(
    root: &Path,
    found: &Found,
    outputs: &Outputs,
    images: &Images,
    tools: &Tools,
    target: &Target,
) -> Result<Shipped, Error> {
    let name = found.spec.name.clone();
    let rendered = workers::render_with(found, outputs, Some(&images.images)).map_err(|p| {
        Error::Invalid(
            p.into_iter()
                .map(|message| crate::Problem {
                    path: format!("{}/{}", found.dir, workers::SPEC),
                    message,
                })
                .collect(),
        )
    })?;
    let dir = workers::write(root, found, &rendered, "pipeline")?;
    let from = serving(&deployments(target, &name)?);
    workers::migrate(found, &dir, tools)?;
    workers::wrangler_deploy(found, &dir, tools)?;
    let to = serving(&deployments(target, &name)?)
        .ok_or_else(|| tool(format!("{name}: no version serves after the deploy")))?;
    let start = Instant::now();
    let mut last = String::from("no answer yet");
    while start.elapsed() < HEALTH_WAIT {
        match health(target) {
            Ok(v) if healthy(&v, &to) => return Ok(Shipped { name, from, to }),
            Ok(v) => last = format!("/healthz answered {v}"),
            Err(e) => last = e,
        }
        sleep(Duration::from_secs(5));
    }
    let Some(back) = from.clone() else {
        return Err(tool(format!(
            "{name} {to} failed its health check ({last}); there was no earlier version to roll back to"
        )));
    };
    target
        .api
        .post(
            &format!(
                "/accounts/{}/workers/scripts/{name}/deployments",
                target.account
            ),
            &json!({ "strategy": "percentage", "versions": [{ "version_id": back, "percentage": 100 }] }),
        )
        .map_err(|e| tool(format!("{name} {to} failed its health check ({last}), and the rollback to {back} failed: {e}")))?;
    Err(tool(format!(
        "{name} {to} failed its health check ({last}); rolled back to {back}"
    )))
}
