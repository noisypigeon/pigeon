//! Worker deploys from the pipeline, with no Docker and no state passphrase: outputs from a
//! snapshot, images by registry reference, then health and rollback; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`. A deploy that
//! changes no object class and no container is version blue-green: green at 0% beside blue, an
//! override smoke, then 100%; see `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`,
//! step 2's spike.

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
    /// What was done, step by step.
    pub steps: Vec<String>,
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

/// `GET https://<host><path>` with the Access service token, to `version` when named; its JSON.
fn get(t: &Target, name: &str, path: &str, version: Option<&str>) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut req = agent
        .get(&format!("https://{}{path}", t.host))
        .header("CF-Access-Client-Id", t.access.0)
        .header("CF-Access-Client-Secret", t.access.1);
    if let Some(v) = version {
        req = req.header(OVERRIDE_HEADER, &format!("{name}=\"{v}\""));
    }
    let mut r = req
        .call()
        .map_err(|_| format!("the request for {path} failed"))?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    serde_json::from_str(&body).map_err(|_| format!("{path} answered {status}, not JSON"))
}

fn deploy(t: &Target, name: &str, body: &Value) -> Result<Value, String> {
    t.api.post(
        &format!("/accounts/{}/workers/scripts/{name}/deployments", t.account),
        body,
    )
}

/// Waits up to `wait` for `path` to pass `check`; the last answer when it never does.
fn wait_for(
    t: &Target,
    name: &str,
    path: &str,
    version: Option<&str>,
    wait: Duration,
    check: impl Fn(&Value) -> bool,
) -> Result<(), String> {
    let start = Instant::now();
    let mut last = String::from("no answer yet");
    while start.elapsed() < wait {
        match get(t, name, path, version) {
            Ok(v) if check(&v) => return Ok(()),
            Ok(v) => last = format!("{path} answered {v}"),
            Err(e) => last = e,
        }
        sleep(Duration::from_secs(5));
    }
    Err(last)
}

/// Deploys `found` from the checkout at `root`: renders it with `outputs` and `images`, applies
/// its migrations, and deploys it whole when [`whole_deploy`] names a reason (`whole`), else as
/// version blue-green; then waits for the new version's health, and on a failed check rolls back
/// to the version that served before.
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
    whole: impl FnOnce(Option<&str>) -> Option<String>,
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
    let mut steps = Vec::new();
    workers::migrate(found, &dir, tools)?;
    let to = match (whole(from.as_deref()), from.as_deref()) {
        (None, Some(blue)) => {
            steps.push("version blue-green".to_string());
            blue_green(&name, blue, &dir, tools, target, &mut steps)?
        }
        (why, _) => {
            steps.push(format!(
                "a whole deploy: {}",
                why.unwrap_or_else(|| "no version serves yet".into())
            ));
            workers::wrangler_deploy(found, &dir, tools)?;
            let to = serving(&deployments(target, &name)?)
                .ok_or_else(|| tool(format!("{name}: no version serves after the deploy")))?;
            match deploy(target, &name, &deployment(&[(&to, 100)], false)) {
                Ok(_) => steps.push(format!("objects take {to} within {CODE_UPDATE_DELAY}s")),
                Err(e) => steps.push(format!("the code update strategy wasn't set: {e}")),
            }
            to
        }
    };
    let last = match wait_for(target, &name, "/healthz", None, HEALTH_WAIT, |v| {
        healthy(v, &to)
    }) {
        Ok(()) => {
            steps.push(format!("{to} is healthy"));
            return Ok(Shipped {
                name,
                from,
                to,
                steps,
            });
        }
        Err(last) => last,
    };
    let Some(back) = from.clone() else {
        return Err(tool(format!(
            "{name} {to} failed its health check ({last}); there was no earlier version to roll back to"
        )));
    };
    deploy(target, &name, &deployment(&[(&back, 100)], true)).map_err(|e| {
        tool(format!(
            "{name} {to} failed its health check ({last}), and the rollback to {back} failed: {e}"
        ))
    })?;
    Err(tool(format!(
        "{name} {to} failed its health check ({last}); rolled back to {back}"
    )))
}

/// Uploads green beside `blue`, deploys it at 0%, smokes it through the override header, then
/// moves it to 100%; green's version. A failed smoke leaves `blue` alone at 100%.
fn blue_green(
    name: &str,
    blue: &str,
    dir: &Path,
    tools: &Tools,
    target: &Target,
    steps: &mut Vec<String>,
) -> Result<String, Error> {
    let out = tools.run(
        &tools.wrangler,
        &["versions", "upload", "-c", "wrangler.jsonc"]
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>(),
        dir,
        None,
    )?;
    let green = uploaded_version(&out)
        .ok_or_else(|| tool(format!("{name}: wrangler versions upload named no version")))?;
    steps.push(format!("uploaded {green}"));
    deploy(
        target,
        name,
        &deployment(&[(blue, 100), (&green, 0)], false),
    )
    .map_err(|e| {
        tool(format!(
            "{name}: {green} wasn't deployed at 0% beside {blue}: {e}"
        ))
    })?;
    steps.push(format!("{green} at 0% beside {blue}"));
    let smoked = wait_for(
        target,
        name,
        "/healthz/smoke",
        Some(&green),
        HEALTH_WAIT,
        |v| smoke_passed(v, &green),
    );
    if let Err(last) = smoked {
        let back = deploy(target, name, &deployment(&[(blue, 100)], true));
        return Err(tool(format!(
            "{name} {green} failed its smoke ({last}) and never took traffic; {}",
            match back {
                Ok(_) => format!("{blue} serves alone"),
                Err(e) => format!("putting {blue} back alone failed: {e}"),
            }
        )));
    }
    steps.push(format!("{green} passed its smoke under the override"));
    deploy(target, name, &deployment(&[(&green, 100)], false))
        .map_err(|e| tool(format!("{name}: {green} wasn't moved to 100%: {e}")))?;
    steps.push(format!(
        "{green} at 100%; objects take it within {CODE_UPDATE_DELAY}s"
    ));
    Ok(green)
}
