//! The deploy job's Workers: those whose inputs changed, each shipped with `cobot::ship` from
//! the snapshots and credentials the job was given; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`. Then `pigeon`,
//! built and stored for job Containers; see
//! `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §3.

use std::fmt::Write as _;
use std::path::Path;

use cobot::ship::{self, Images, Outputs, Target};
use cobot::workers::{self, Tools};

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
    let names = cobot::scope::workers(dir, files).map_err(|e| e.to_string())?;
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
    let outputs: Outputs = serde_json::from_str(&get(ship::OUTPUTS_VAR))
        .map_err(|e| format!("{} is not JSON outputs: {e}", ship::OUTPUTS_VAR))?;
    let mut images: Images = serde_json::from_str(&get(ship::IMAGES_VAR))
        .map_err(|e| format!("{} is not JSON images: {e}", ship::IMAGES_VAR))?;
    let since_image = cobot::ci::changed_since(dir, &images.source, &spec.sha).map_err(|e| {
        format!(
            "the images' commit {} isn't in main's history: {e}",
            images.source
        )
    })?;
    let token = get(ship::TOKEN_VAR);
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let mut tools = Tools::from(|_| None);
    tools.env = vec![
        ("CLOUDFLARE_ACCOUNT_ID".into(), account.clone()),
        ("CLOUDFLARE_API_TOKEN".into(), token.clone()),
    ];
    tools.secrets = vec![token.clone(), get(ship::ACCESS_SECRET_VAR)];
    let blocks = cobot::scope::image_inputs(&since_image);
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
        let found = workers::select(dir, std::slice::from_ref(&name))
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("no Worker `{name}`"))?;
        let s = ship::ship(dir, &found, &outputs, &images, &tools, &target)
            .map_err(|e| e.to_string())?;
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

/// The command that builds the `pigeon` jobs run.
pub const BUILD: [&str; 9] = [
    "cargo",
    "build",
    "--release",
    "--locked",
    "-p",
    "pigeon",
    "--no-default-features",
    "--features",
    "backplane",
];

/// Builds `pigeon` from the checkout at `dir` when the workspace changed among `files`, and
/// stores it at [`cobot::ship_wire::build_key`] in the [`BUILDS_BINDING`] bucket, with S3
/// credentials from the deploy token; the build, or none when nothing changed.
///
/// # Errors
///
/// Why it wasn't built or stored: a missing variable, a failed build or upload.
pub fn build(
    spec: &JobSpec,
    dir: &Path,
    files: Option<&[String]>,
    log: &mut String,
) -> Result<Option<Build>, String> {
    if !cobot::scope::builds(files) {
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
    let reference = workers::discover(dir)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find_map(|f| f.spec.bindings.r2.get(BUILDS_BINDING).cloned())
        .ok_or_else(|| format!("no Worker binds {BUILDS_BINDING}, where builds are stored"))?;
    let bucket = workers::resolve(&reference, &outputs)?;
    let workspace = dir.join(cobot::scope::WORKSPACE);
    let argv: Vec<String> = BUILD.iter().map(|s| (*s).to_string()).collect();
    let started = std::time::Instant::now();
    let (ok, out) = cobot::ci::exec(&workspace, &argv, cobot::ci::TIMEOUT, &spec.token);
    let took = started.elapsed().as_secs();
    if !ok {
        let _ = writeln!(log, "{}", crate::control::tail(&out));
        return Err(format!("pigeon's release build failed after {took}s"));
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), std::path::PathBuf::from);
    let binary = std::fs::read(target.join("release/pigeon"))
        .map_err(|e| format!("the built pigeon can't be read: {e}"))?;
    let sha256 = cobot::s3::sha256_hex(&binary);
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let creds = cobot::s3::Credentials {
        access_key: api.token_id(&account)?,
        secret_key: cobot::s3::sha256_hex(token.as_bytes()),
    };
    let store = cobot::s3::Bucket {
        endpoint: cobot::sites::endpoint(&account),
        name: bucket.clone(),
        creds,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let key = cobot::ship_wire::build_key(&spec.sha);
    store
        .put(&key, &binary, "application/octet-stream", now)
        .map_err(|e| {
            format!("storing {key} failed: {e}")
                .replace(&bucket, "…")
                .replace(&account, "…")
        })?;
    let _ = writeln!(
        log,
        "stored {key}: built in {took}s, {} bytes, sha256 {sha256}",
        binary.len()
    );
    Ok(Some(Build {
        sha: spec.sha.clone(),
        sha256,
    }))
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
    api: &cobot::cloudflare::Api,
    account: &str,
    tools: &Tools,
    log: &mut String,
) -> Result<Images, String> {
    let failed = |e: cobot::Error| format!("needs image: building it failed: {e}");
    let work = std::env::temp_dir().join("pigeon-images");
    let _ = std::fs::remove_dir_all(&work);
    let builder = cobot::imagebuild::fetch(&work).map_err(failed)?;
    let tag: String = spec.sha.chars().take(12).collect();
    let mut refs = images.images.clone();
    for name in names {
        let found = workers::select(dir, std::slice::from_ref(name))
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("no Worker `{name}`"))?;
        if found.spec.containers.is_empty() {
            continue;
        }
        refs.extend(
            cobot::imagebuild::build_and_push(
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
