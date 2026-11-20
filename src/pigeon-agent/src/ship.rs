//! The deploy job's Workers: those whose inputs changed, each shipped with `crate::workers::ship` from
//! the snapshots and credentials the job was given; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`. Then `pigeon`,
//! built and stored for job Containers; see
//! `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §3.

use std::fmt::Write as _;
use std::path::Path;

use crate::workers::ship::{self, Images, Outputs, Target};
use crate::workers::{self, Tools};

use crate::control::{Build, JobSpec};

/// The variables a Worker deploy needs from the job's environment.
pub const NEEDED: [&str; 6] = [
    ship::TOKEN_VAR,
    ship::OUTPUTS_VAR,
    ship::IMAGES_VAR,
    ship::ACCESS_ID_VAR,
    ship::ACCESS_SECRET_VAR,
    ship::HOST_VAR,
];

/// Why the deploy of `spec.sha` is superseded: `main` has moved past it. None when `main` is
/// still there or its head can't be read; see the deploys ADR's 2026-11-16 amendment.
#[must_use]
pub fn superseded(spec: &JobSpec, dir: &Path) -> Option<String> {
    crate::checks::ci::remote_head(dir, &spec.remote, &spec.token, &spec.branch)
        .ok()
        .filter(|head| *head != spec.sha)
        .map(|head| {
            format!(
                "main moved on to {head}; the deploy of {} is superseded",
                spec.sha
            )
        })
}

/// What `/healthz` is asked of: the front host, and each colour's `(worker, host)`.
type Probes = (String, Vec<(String, String)>);

/// The hosts to probe, from the manifest and the job's `PUBLIC_HOST`: the `pigeon/platform`
/// unit's first domain as the front, its Worker at `public_host`, and each `pigeon/color` unit's
/// Worker at its own host. `None` without a platform unit.
fn probes(m: &crate::infra::manifest::Manifest, public_host: &str) -> Option<Probes> {
    let platform = m.units.values().find(|e| e.kind == "pigeon/platform")?;
    let front = platform.settings["domains"][0].as_str()?.to_string();
    let mut colors = Vec::new();
    if let Some(w) = platform.settings["worker"].as_str() {
        colors.push((w.to_string(), public_host.to_string()));
    }
    for e in m.units.values().filter(|e| e.kind == "pigeon/color") {
        if let (Some(w), Some(h)) = (e.settings["worker"].as_str(), e.settings["host"].as_str()) {
            colors.push((w.to_string(), h.to_string()));
        }
    }
    Some((front, colors))
}

/// Each `{ host }` route of the Worker specs at `dir`, bound to `deployment`, as `(worker, host)`.
fn route_hosts(dir: &Path, deployment: Option<&str>) -> Vec<(String, String)> {
    let Ok(found) = crate::workers::discover(dir) else {
        return Vec::new();
    };
    found
        .iter()
        .filter_map(|f| crate::workers::bound(dir, f, deployment).ok())
        .flat_map(|b| {
            let name = b.spec.name.clone();
            b.spec
                .routes
                .iter()
                .filter_map(|r| r.host.clone())
                .map(move |h| (name.clone(), h))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Why a `/healthz` answer of `status` with `body` isn't up: the status, and the names of the
/// health fields that fail; `None` when it is up. Never a value.
#[must_use]
pub fn down_reason(status: u16, body: &str) -> Option<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Some(format!("status {status}, not JSON"));
    };
    if status == 200 && v["ok"].as_bool() == Some(true) {
        return None;
    }
    let mut failing: Vec<&str> = ["d1", "durable_object"]
        .into_iter()
        .filter(|k| v[*k].as_bool() != Some(true))
        .collect();
    if v["objects"]["instance"] != v["objects"]["expected"] {
        failing.push("objects");
    }
    if v["version"].as_str().is_none_or(str::is_empty) {
        failing.push("version");
    }
    Some(if failing.is_empty() {
        format!("status {status}")
    } else {
        format!("status {status}: {}", failing.join(", "))
    })
}

/// How an up host was reached: `direct`, or `via front` with the colour pigeon-front forwarded to,
/// from its [`FORWARDED`] header (the blue-green ADR's exact-host route amendment).
#[must_use]
pub fn reached(forwarded: Option<&str>) -> String {
    match forwarded {
        Some(c) if !c.is_empty() => format!("via front to {c}"),
        _ => "direct".into(),
    }
}

/// pigeon-front's header naming the colour it forwarded an answer to.
pub const FORWARDED: &str = "x-pigeon-front";

/// Whether `GET https://<host>/healthz` answers 200 with `ok` true, in up to 3 tries 10 s apart:
/// how it was reached, as [`reached`] says, or why the last try wasn't up, as [`down_reason`] says.
fn probe(host: &str, access: (&str, &str)) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();
    let mut why = String::from("no answer");
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        let answer = agent
            .get(&format!("https://{host}/healthz"))
            .header("CF-Access-Client-Id", access.0)
            .header("CF-Access-Client-Secret", access.1)
            .call();
        why = match answer {
            Ok(mut r) => {
                let status = r.status().as_u16();
                let via = reached(r.headers().get(FORWARDED).and_then(|v| v.to_str().ok()));
                let body = r.body_mut().read_to_string().unwrap_or_default();
                match down_reason(status, &body) {
                    None => return Ok(via),
                    Some(w) => w,
                }
            }
            Err(_) => "no answer".into(),
        };
    }
    Err(why)
}

/// What serves once the deploy ended, by `/healthz` on the front host and on each colour's own
/// host; `None` (failing closed) when the job lacks what it takes to ask, and the log says why;
/// see .claude/decisions/2026-11-19-security-auto-held-when-platform-down.md §1.
pub fn serving(
    spec: &JobSpec,
    dir: &Path,
    log: &mut String,
) -> Option<crate::core::deploy::serving::Serving> {
    use crate::wire::ship_wire as w;
    let var = |k: &str| {
        spec.env
            .get(k)
            .filter(|v| !v.is_empty())
            .map(String::as_str)
    };
    let (Some(public), Some(id), Some(secret)) = (
        var(w::HOST_VAR),
        var(w::ACCESS_ID_VAR),
        var(w::ACCESS_SECRET_VAR),
    ) else {
        let _ = writeln!(
            log,
            "serving: not probed: the job lacks a host or Access token"
        );
        return None;
    };
    let Some((m, _)) = crate::infra::manifest::load(dir, dir) else {
        let _ = writeln!(log, "serving: not probed: no manifest");
        return None;
    };
    let Some((front, colors)) = probes(&m, public) else {
        let _ = writeln!(
            log,
            "serving: not probed: no pigeon/platform unit with a domain"
        );
        return None;
    };
    let say = |r: &Result<String, String>| match r {
        Ok(via) => format!("up ({via})"),
        Err(why) => format!("down ({why})"),
    };
    let mut out = crate::core::deploy::serving::Serving {
        lease: var(w::LEASE_VAR)
            .unwrap_or(crate::core::lease::FIRST_COLOR)
            .to_string(),
        ..Default::default()
    };
    let probed: Vec<String> = colors.iter().map(|(_, h)| h.clone()).collect();
    for (worker, host) in colors {
        let r = probe(&host, (id, secret));
        let _ = writeln!(log, "serving: {worker} {host}: {}", say(&r));
        out.colors.insert(worker, r.is_ok());
    }
    // Each Worker's other own hosts, by its exact-host routes, logged only.
    for (worker, host) in route_hosts(dir, var(crate::workers::DEPLOYMENT_VAR)) {
        if probed.contains(&host) {
            continue;
        }
        let r = probe(&host, (id, secret));
        let _ = writeln!(log, "serving: {worker} {host}: {}", say(&r));
    }
    let r = probe(&front, (id, secret));
    let _ = writeln!(log, "serving: front {front}: {}", say(&r));
    out.front = r.is_ok();
    Some(out)
}

/// Deploys the Workers whose inputs changed among `files` from the checkout at `dir`; each as
/// `<name>: <from> → <to>`. When the job image's inputs changed since its commit, it is built and
/// pushed first, and set in `built`, even if a later step fails.
///
/// # Errors
///
/// Why no Worker, or not every one, was deployed: a missing variable, a failed image build, or a
/// failed step, after any rollback.
pub fn workers(
    spec: &JobSpec,
    dir: &Path,
    files: Option<&[String]>,
    log: &mut String,
    built: &mut Option<crate::control::Images>,
) -> Result<Vec<String>, String> {
    let names = crate::checks::scope::workers(dir, files).map_err(|e| e.to_string())?;
    if names.is_empty() {
        log.push_str("no Worker to deploy\n");
        return Ok(Vec::new());
    }
    let _ = writeln!(log, "Workers to deploy: {}", names.join(", "));
    let var = |k: &str| spec.env.get(k).filter(|v| !v.is_empty()).cloned();
    let missing: Vec<&str> = NEEDED
        .iter()
        .copied()
        .filter(|k| var(k).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Workers aren't deployed from the pipeline yet: the control plane lacks {}",
            missing.join(", ")
        ));
    }
    let get = |k: &str| var(k).unwrap_or_default();
    let deployment = var(workers::DEPLOYMENT_VAR);
    let mut outputs: Outputs = serde_json::from_str(&get(ship::OUTPUTS_VAR))
        .map_err(|e| format!("{} is not JSON outputs: {e}", ship::OUTPUTS_VAR))?;
    workers::site_buckets(dir, &mut outputs);
    let mut images: Images = serde_json::from_str(&get(ship::IMAGES_VAR))
        .map_err(|e| format!("{} is not JSON images: {e}", ship::IMAGES_VAR))?;
    let since_image =
        crate::checks::ci::changed_since(dir, &images.source, &spec.sha).map_err(|e| {
            format!(
                "the images' commit {} isn't in main's history: {e}",
                images.source
            )
        })?;
    let token = get(ship::TOKEN_VAR);
    let api = crate::workers::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let mut tools = Tools::from(|_| None);
    tools.env = vec![
        ("CLOUDFLARE_ACCOUNT_ID".into(), account.clone()),
        ("CLOUDFLARE_API_TOKEN".into(), token.clone()),
    ];
    tools.secrets = vec![token.clone(), get(ship::ACCESS_SECRET_VAR)];
    let mut blocks = crate::checks::scope::image_inputs(&since_image);
    // A container class new since the images were built has none; it is built like a changed
    // image. See the job-images ADR's 2026-11-16 amendment.
    blocks.extend(
        unimaged(dir, &names, deployment.as_deref(), &images.images)?
            .into_iter()
            .map(|k| format!("no image for {k}")),
    );
    if !blocks.is_empty() {
        let _ = writeln!(
            log,
            "the job image is behind {}: building it",
            blocks.join(", ")
        );
        images = build_images(spec, dir, &names, &images, &api, &account, &tools, log)?;
        *built = Some(crate::control::Images {
            source: images.source.clone(),
            images: images.images.clone(),
        });
    }
    tools.secrets.extend(ship::secrets_of(&outputs, &images));
    let host = get(ship::HOST_VAR);
    let (id, secret) = (get(ship::ACCESS_ID_VAR), get(ship::ACCESS_SECRET_VAR));
    let target = Target {
        api: &api,
        account: &account,
        host: &host,
        access: (&id, &secret),
    };
    let mut shipped = Vec::new();
    for name in names {
        let found = workers::select_bound(dir, std::slice::from_ref(&name), deployment.as_deref())
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("no Worker `{name}`"))?;
        let rebuilt = built.is_some();
        let whole = |from: Option<&str>| ship::whole_deploy(&found.dir, files, rebuilt, from);
        let s = ship::ship(dir, &found, &outputs, &images, &tools, &target, whole)
            .map_err(|e| e.to_string())?;
        for step in &s.steps {
            let _ = writeln!(log, "{}: {step}", s.name);
        }
        let line = format!(
            "{}: {} → {}",
            s.name,
            s.from.as_deref().unwrap_or("none"),
            s.to
        );
        let _ = writeln!(log, "deployed {line}");
        shipped.push(line);
    }
    Ok(shipped)
}

/// The R2 binding, in some Worker's spec, of the bucket builds are stored in and served from.
pub const BUILDS_BINDING: &str = "CI_LOGS";

/// The command that builds the `pigeon` jobs run, by its binary's name, so it builds whichever
/// package holds it.
pub const BUILD: [&str; 9] = [
    "cargo",
    "build",
    "--release",
    "--locked",
    "--bin",
    "pigeon",
    "--no-default-features",
    "--features",
    "agent",
];

/// The command that builds the judge the bound job judges with, by its binary's name `bin`, so it
/// builds whichever package holds it.
#[must_use]
pub fn judge_build(bin: &str) -> [&str; 6] {
    ["cargo", "build", "--release", "--locked", "--bin", bin]
}

/// The first of [`crate::wire::ship_wire::JUDGE_BINARIES`] that a member of the workspace at
/// `ws` declares, as a `[[bin]]` or a package with `src/main.rs`; else the old name.
#[must_use]
pub fn judge_binary(ws: &Path) -> &'static str {
    let manifest = |dir: &Path| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .ok()
            .and_then(|t| t.parse::<toml::Value>().ok())
    };
    let members: Vec<String> = manifest(ws)
        .and_then(|m| {
            let list = m.get("workspace")?.get("members")?.as_array()?.clone();
            Some(
                list.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
            )
        })
        .unwrap_or_default();
    let mut dirs = Vec::new();
    for m in &members {
        match m.trim_end_matches('/').strip_suffix("/*") {
            Some(parent) => {
                let mut found: Vec<_> = std::fs::read_dir(ws.join(parent))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .collect();
                found.sort();
                dirs.extend(found);
            }
            None => dirs.push(ws.join(m)),
        }
    }
    let mut bins = std::collections::BTreeSet::new();
    for dir in dirs {
        let Some(m) = manifest(&dir) else { continue };
        let declared: Vec<String> = m
            .get("bin")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|b| b.get("name")?.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if declared.is_empty()
            && dir.join("src/main.rs").is_file()
            && let Some(name) = m.get("package").and_then(|p| p.get("name")?.as_str())
        {
            bins.insert(name.to_string());
        }
        bins.extend(declared);
    }
    crate::wire::ship_wire::JUDGE_BINARIES
        .into_iter()
        .find(|b| bins.contains(*b))
        .unwrap_or(crate::wire::ship_wire::STORED_JUDGE)
}

/// Builds `pigeon` from the checkout at `dir` when `files` changed its crates and the judge when
/// `judge_files` changed its crate, and
/// stores each at [`crate::wire::ship_wire::build_key`] in the [`BUILDS_BINDING`] bucket, with S3
/// credentials from the deploy token; the build, or none when nothing changed.
///
/// # Errors
///
/// Why it wasn't built or stored: a missing variable, a failed build or upload.
#[expect(clippy::too_many_lines, reason = "one step per stored binary")]
pub fn build(
    spec: &JobSpec,
    dir: &Path,
    files: Option<&[String]>,
    judge_files: Option<&[String]>,
    log: &mut String,
) -> Result<Option<Build>, String> {
    let pigeon = crate::checks::scope::builds(files);
    let judge = crate::checks::scope::builds_judge(judge_files);
    if !pigeon && !judge {
        log.push_str("no Rust change: jobs keep the last build\n");
        return Ok(None);
    }
    let var = |k: &str| spec.env.get(k).filter(|v| !v.is_empty()).cloned();
    let (Some(token), Some(outputs)) = (var(ship::TOKEN_VAR), var(ship::OUTPUTS_VAR)) else {
        return Err(format!(
            "pigeon isn't built from the pipeline yet: the control plane lacks {} or {}",
            ship::TOKEN_VAR,
            ship::OUTPUTS_VAR
        ));
    };
    let outputs: Outputs = serde_json::from_str(&outputs)
        .map_err(|e| format!("{} is not JSON outputs: {e}", ship::OUTPUTS_VAR))?;
    let deployment = var(workers::DEPLOYMENT_VAR);
    let mut reference = None;
    for f in workers::discover(dir).map_err(|e| e.to_string())? {
        // A spec that doesn't bind here, such as a colour spec with several units, holds no bucket.
        let Ok(f) = workers::bound(dir, &f, deployment.as_deref()) else {
            continue;
        };
        if let Some(r) = f.spec.bindings.r2.get(BUILDS_BINDING) {
            reference = Some(r.clone());
            break;
        }
    }
    let reference = reference
        .ok_or_else(|| format!("no Worker binds {BUILDS_BINDING}, where builds are stored"))?;
    let bucket = workers::resolve(&reference, &outputs)?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| dir.join("target"), std::path::PathBuf::from);
    let compile = |argv: &[&str], name: &str, log: &mut String| {
        let argv: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
        let started = std::time::Instant::now();
        let (ok, out) =
            crate::checks::ci::exec(dir, &argv, crate::checks::ci::TIMEOUT, &spec.token);
        let took = started.elapsed().as_secs();
        if !ok {
            let _ = writeln!(log, "{}", crate::control::tail(&out));
            return Err(format!("{name}'s release build failed after {took}s"));
        }
        let binary = std::fs::read(target.join("release").join(name))
            .map_err(|e| format!("the built {name} can't be read: {e}"))?;
        Ok((binary, took))
    };
    let built_pigeon = if pigeon {
        let (binary, took) = compile(&BUILD, "pigeon", log)?;
        let sha256 = crate::infra::s3::sha256_hex(&binary);
        Some((binary, took, sha256))
    } else {
        log.push_str("no change to pigeon's crates: jobs keep the last pigeon\n");
        None
    };
    let built_judge = if judge {
        let name = judge_binary(dir);
        let (bytes, took) = compile(&judge_build(name), name, log)?;
        let sha256 = crate::infra::s3::sha256_hex(&bytes);
        Some((bytes, took, sha256))
    } else {
        log.push_str("no change to the judge's crate: jobs keep the last judge\n");
        None
    };
    let api = crate::workers::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let creds = crate::infra::s3::Credentials {
        access_key: api.token_id(&account)?,
        secret_key: crate::infra::s3::sha256_hex(token.as_bytes()),
    };
    let store = crate::infra::s3::Bucket {
        endpoint: crate::sites::endpoint(&account),
        name: bucket.clone(),
        creds,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut uploads = Vec::new();
    if let Some((bytes, took, sum)) = &built_pigeon {
        uploads.push((
            crate::wire::ship_wire::build_key(&spec.sha),
            bytes,
            *took,
            sum,
        ));
    }
    if let Some((bytes, took, sum)) = &built_judge {
        uploads.push((
            crate::wire::ship_wire::judge_key(&spec.sha),
            bytes,
            *took,
            sum,
        ));
    }
    for (key, bytes, took, sum) in uploads {
        store
            .put(&key, bytes, "application/octet-stream", now)
            .map_err(|e| {
                format!("storing {key} failed: {e}")
                    .replace(&bucket, "…")
                    .replace(&account, "…")
            })?;
        let _ = writeln!(
            log,
            "stored {key}: built in {took}s, {} bytes, sha256 {sum}",
            bytes.len()
        );
    }
    Ok(Some(Build {
        sha: spec.sha.clone(),
        sha256: built_pigeon.map(|(_, _, s)| s).unwrap_or_default(),
        judge_sha256: built_judge.map(|(_, _, s)| s),
        judge_sha: None,
    }))
}

/// The image keys, `<worker>/<class>`, of `names`' containers that `images` lacks.
fn unimaged(
    dir: &Path,
    names: &[String],
    deployment: Option<&str>,
    images: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for name in names {
        let found = workers::select_bound(dir, std::slice::from_ref(name), deployment)
            .map_err(|e| e.to_string())?;
        for f in found {
            for c in &f.spec.containers {
                let key = workers::image_key(&f.spec, &c.class);
                if !images.contains_key(&key) {
                    out.push(key);
                }
            }
        }
    }
    Ok(out)
}

/// The job images of the Workers `names` that have containers, built from the checkout at `dir`
/// and pushed as of `spec.sha`, over `images`; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §4.
#[allow(clippy::too_many_arguments)]
fn build_images(
    spec: &JobSpec,
    dir: &Path,
    names: &[String],
    images: &Images,
    api: &crate::workers::cloudflare::Api,
    account: &str,
    tools: &Tools,
    log: &mut String,
) -> Result<Images, String> {
    let failed = |e: crate::infra::Error| format!("needs image: building it failed: {e}");
    let deployment = spec
        .env
        .get(workers::DEPLOYMENT_VAR)
        .filter(|v| !v.is_empty())
        .cloned();
    let work = std::env::temp_dir().join("pigeon-images");
    let _ = std::fs::remove_dir_all(&work);
    let builder = crate::workers::imagebuild::fetch(&work).map_err(failed)?;
    let tag: String = spec.sha.chars().take(12).collect();
    let mut refs = images.images.clone();
    for name in names {
        let found = workers::select_bound(dir, std::slice::from_ref(name), deployment.as_deref())
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("no Worker `{name}`"))?;
        if found.spec.containers.is_empty() {
            continue;
        }
        refs.extend(
            crate::workers::imagebuild::build_and_push(
                dir,
                &found,
                &tag,
                account,
                api,
                workers::LOGIN_MINUTES,
                &builder,
                tools,
                &work,
                log,
            )
            .map_err(failed)?,
        );
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(Images {
        source: spec.sha.clone(),
        images: refs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_down_host_is_named_by_its_status_and_failing_fields_only() {
        let up = r#"{"ok":true,"version":"v","d1":true,"durable_object":true,"objects":{"instance":"a","expected":"a"}}"#;
        assert_eq!(down_reason(200, up), None);
        let stale = r#"{"ok":false,"version":"v","d1":true,"durable_object":true,"objects":{"instance":"a","expected":"b"}}"#;
        assert_eq!(
            down_reason(503, stale).as_deref(),
            Some("status 503: objects")
        );
        let bare = r#"{"ok":false,"version":"","d1":false,"durable_object":true,"objects":{}}"#;
        assert_eq!(
            down_reason(503, bare).as_deref(),
            Some("status 503: d1, version")
        );
        assert_eq!(
            down_reason(302, "<html>sign in</html>").as_deref(),
            Some("status 302, not JSON")
        );
        assert_eq!(
            down_reason(401, "unauthorized: no token").as_deref(),
            Some("status 401, not JSON")
        );
    }

    fn tree(crates: &[(&str, &str, bool)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"src/*\"]\n",
        )
        .expect("workspace");
        for (name, manifest, main) in crates {
            let c = dir.path().join("src").join(name);
            std::fs::create_dir_all(c.join("src")).expect("crate");
            std::fs::write(c.join("Cargo.toml"), manifest).expect("manifest");
            if *main {
                std::fs::write(c.join("src/main.rs"), "fn main() {}\n").expect("main");
            }
        }
        dir
    }

    #[test]
    fn the_judge_is_built_under_the_name_the_tree_declares() {
        let stored = crate::wire::ship_wire::STORED_JUDGE;
        let old = format!(
            "[package]\nname = \"pigeon-eviction\"\n\n[[bin]]\nname = \"{stored}\"\npath = \"src/main.rs\"\n"
        );
        let new = "[package]\nname = \"pigeon-eviction\"\n\n[[bin]]\nname = \"pigeon-eviction\"\npath = \"src/main.rs\"\n";
        let other = "[package]\nname = \"pigeon\"\n";
        let t = tree(&[
            ("pigeon-eviction", old.as_str(), true),
            ("pigeon-harness", other, true),
        ]);
        assert_eq!(judge_binary(t.path()), stored);
        let t = tree(&[
            ("pigeon-eviction", new, true),
            ("pigeon-harness", other, true),
        ]);
        assert_eq!(judge_binary(t.path()), "pigeon-eviction");
        let implicit = "[package]\nname = \"pigeon-eviction\"\n";
        let t = tree(&[("pigeon-eviction", implicit, true)]);
        assert_eq!(judge_binary(t.path()), "pigeon-eviction");
        let t = tree(&[("pigeon-harness", other, true)]);
        assert_eq!(judge_binary(t.path()), stored);
        assert_eq!(
            judge_build("pigeon-eviction"),
            [
                "cargo",
                "build",
                "--release",
                "--locked",
                "--bin",
                "pigeon-eviction"
            ]
        );
    }
}

#[cfg(test)]
mod serving_tests {
    use super::*;
    use crate::infra::manifest::{Entry, Manifest, Role};

    fn unit(kind: &str, settings: serde_json::Value) -> Entry {
        Entry {
            kind: kind.into(),
            schema: serde_json::Value::Null,
            role: Role::Plain,
            settings,
            api: None,
        }
    }

    #[test]
    fn probes_the_front_and_each_colour() {
        let m = Manifest {
            root: ".".into(),
            name: "pigeon".into(),
            units: [
                (
                    "platform/blue".to_string(),
                    unit(
                        "pigeon/platform",
                        serde_json::json!({"worker": "pigeon-control", "domains": ["platform.example.com"]}),
                    ),
                ),
                (
                    "color/green".to_string(),
                    unit(
                        "pigeon/color",
                        serde_json::json!({"worker": "pigeon-green", "host": "green.example.com"}),
                    ),
                ),
            ]
            .into(),
            moves: Vec::new(),
            linked: std::collections::BTreeSet::default(),
        };
        let (front, colors) = probes(&m, "blue.example.com").expect("probes");
        assert_eq!(front, "platform.example.com");
        assert_eq!(
            colors,
            vec![
                ("pigeon-control".to_string(), "blue.example.com".to_string()),
                ("pigeon-green".to_string(), "green.example.com".to_string()),
            ]
        );
        let none = Manifest {
            units: std::collections::BTreeMap::default(),
            ..m
        };
        assert!(probes(&none, "x").is_none());
    }
}
