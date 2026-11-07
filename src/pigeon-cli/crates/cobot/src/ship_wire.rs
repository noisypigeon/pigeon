//! The names a pipeline deploy reads, shared by both builds; see
//! `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md` §3, §6.

/// The Worker secret holding the outputs a deploy resolves, as JSON.
pub const OUTPUTS_VAR: &str = "DEPLOY_OUTPUTS";
/// The Worker secret holding the images a deploy references, as JSON.
pub const IMAGES_VAR: &str = "DEPLOY_IMAGES";
/// The account token a Worker deploy runs with.
pub const TOKEN_VAR: &str = "WORKERS_DEPLOY_TOKEN";
/// The health check's Access service token: its client ID.
pub const ACCESS_ID_VAR: &str = "DEPLOYER_ACCESS_ID";
/// See [`ACCESS_ID_VAR`]: its secret.
pub const ACCESS_SECRET_VAR: &str = "DEPLOYER_ACCESS_SECRET";
/// The host the health check asks; a var, not a secret.
pub const HOST_VAR: &str = "PUBLIC_HOST";

/// The Worker secrets only the deploy job receives.
pub const SECRETS: [&str; 5] = [
    TOKEN_VAR,
    OUTPUTS_VAR,
    IMAGES_VAR,
    ACCESS_ID_VAR,
    ACCESS_SECRET_VAR,
];

/// The Access service token job Containers download builds with: its client ID; the Worker
/// lets it read `/builds/` only.
pub const BUILDS_ACCESS_ID_VAR: &str = "BUILDS_ACCESS_ID";
/// See [`BUILDS_ACCESS_ID_VAR`]: its secret.
pub const BUILDS_ACCESS_SECRET_VAR: &str = "BUILDS_ACCESS_SECRET";

/// The R2 key of `main`'s `pigeon` built at `sha`.
#[must_use]
pub fn build_key(sha: &str) -> String {
    format!("builds/{sha}/pigeon")
}

/// The commit of a `/builds/<sha>/pigeon` path: a full, lowercase SHA-1, and nothing else.
#[must_use]
pub fn build_path_sha(path: &str) -> Option<&str> {
    let sha = path.strip_prefix("/builds/")?.strip_suffix("/pigeon")?;
    (sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(sha)
}
