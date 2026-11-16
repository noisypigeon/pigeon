//! Worker deploys from the pipeline, with no Docker and no state passphrase: outputs from a
//! snapshot, images by registry reference, then health and rollback; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`. A deploy that
//! changes no object class and no container is version blue-green: green at 0% beside blue, an
//! override smoke, then 100%; see `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`,
//! step 2's spike.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cobot::Error;
use crate::cobot::cloudflare::Api;
use crate::cobot::workers::{self, Found, Spec, Tools};

pub use crate::cobot::ship_wire::{
    ACCESS_ID_VAR, ACCESS_SECRET_VAR, HOST_VAR, IMAGES_VAR, OUTPUTS_VAR, TOKEN_VAR,
};

pub use crate::core::deploy::ship::*;

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
                .map(|message| crate::cobot::Problem {
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
