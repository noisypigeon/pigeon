//! The deploy job's Workers: those whose inputs changed, each shipped with `cobot::ship` from
//! the snapshots and credentials the job was given; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`.

use std::fmt::Write as _;
use std::path::Path;

use cobot::ship::{self, Images, Outputs, Target};
use cobot::workers::{self, Tools};

use crate::control::JobSpec;

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
/// `<name>: <from> → <to>`.
///
/// # Errors
///
/// Why no Worker, or not every one, was deployed: a missing variable or image, or a failed
/// step, after any rollback.
pub fn workers(
    spec: &JobSpec,
    dir: &Path,
    files: Option<&[String]>,
    log: &mut String,
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
    let images: Images = serde_json::from_str(&get(ship::IMAGES_VAR))
        .map_err(|e| format!("{} is not JSON images: {e}", ship::IMAGES_VAR))?;
    let since_image = cobot::ci::changed_since(dir, &images.source, &spec.sha).map_err(|e| {
        format!(
            "the images' commit {} isn't in main's history: {e}",
            images.source
        )
    })?;
    let (blocks, lags) = cobot::scope::image_inputs(&since_image);
    if !blocks.is_empty() {
        return Err(format!(
            "needs image: {} changed since the images' commit {}; push new images with `pigeon cobot workers images push | pigeon cobot workers secrets put --root . <worker> {}`, then `deploy_retry`",
            blocks.join(", "),
            &images.source[..images.source.len().min(12)],
            ship::IMAGES_VAR
        ));
    }
    if !lags.is_empty() {
        let _ = writeln!(
            log,
            "note: the job image is behind {} file(s) of pigeon's code; it catches up at the next `images push`",
            lags.len()
        );
    }
    let token = get(ship::TOKEN_VAR);
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let mut tools = Tools::from(|_| None);
    tools.env = vec![
        ("CLOUDFLARE_ACCOUNT_ID".into(), account.clone()),
        ("CLOUDFLARE_API_TOKEN".into(), token.clone()),
    ];
    tools.secrets = ship::secrets_of(&outputs, &images);
    tools.secrets.extend([token, get(ship::ACCESS_SECRET_VAR)]);
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
