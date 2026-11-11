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

/// The Worker var naming the stack it was bound to; see `crate::workers::bind`.
pub const DEPLOYMENT_VAR: &str = "DEPLOYMENT";

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

/// The binaries a build stores, by name: `pigeon`, and `pigeon-sentinel`, which the bound job
/// judges with; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`
/// §2.
pub const BUILD_BINARIES: [&str; 2] = ["pigeon", SENTINEL_BINARY];

/// The sentinel binary's name.
pub const SENTINEL_BINARY: &str = "pigeon-sentinel";

/// The variable naming the `pigeon-sentinel` build a job Container downloads.
pub const SENTINEL_URL_VAR: &str = "PIGEON_SENTINEL_URL";
/// The variable holding that build's SHA-256.
pub const SENTINEL_SHA256_VAR: &str = "PIGEON_SENTINEL_SHA256";

/// The test map kept beside a build; see the 2026-11-11 amendment to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
pub const TEST_MAP_FILE: &str = "tests.json";

/// The variable naming the test map a job Container downloads.
pub const TEST_MAP_URL_VAR: &str = "PIGEON_TEST_MAP_URL";
/// The variable holding that map's SHA-256.
pub const TEST_MAP_SHA256_VAR: &str = "PIGEON_TEST_MAP_SHA256";

/// The R2 key of `main`'s `pigeon-sentinel` built at `sha`.
#[must_use]
pub fn sentinel_key(sha: &str) -> String {
    format!("builds/{sha}/{SENTINEL_BINARY}")
}

/// The commit and file of a `/builds/<sha>/<file>` path: a full, lowercase SHA-1 and one of
/// [`BUILD_BINARIES`] or [`TEST_MAP_FILE`], and nothing else.
#[must_use]
pub fn build_path(path: &str) -> Option<(&str, &str)> {
    let (sha, binary) = path.strip_prefix("/builds/")?.split_once('/')?;
    (sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (BUILD_BINARIES.contains(&binary) || binary == TEST_MAP_FILE))
        .then_some((sha, binary))
}

/// The commit of a `/builds/<sha>/pigeon` path: a full, lowercase SHA-1, and nothing else.
#[must_use]
pub fn build_path_sha(path: &str) -> Option<&str> {
    build_path(path)
        .filter(|(_, binary)| *binary == "pigeon")
        .map(|(sha, _)| sha)
}
