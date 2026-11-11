//! The wasm-safe `worker` build: Container dispatch and minting, per the control-plane ADR §1
//! and §5 (`.claude/decisions/2026-11-05-runtime-control-plane-worker.md`). Every request to a
//! run's Container is signed, and every answer is verified before it is read.

use crate::cobot::link;

/// The Container port `serve` listens on.
pub const PORT: u16 = 8080;

/// The environment variable `serve` reads its signing key from.
pub const TOKEN_VAR: &str = "BACKPLANE_TOKEN";

/// The variables cobot reads an `r2://` state's access from; equal to `state.rs`'s and
/// `engines.rs`'s, which a test checks.
pub const STATE_VARS: [&str; 4] = [
    "COBOT_STATE_R2_ENDPOINT",
    "COBOT_STATE_R2_ACCESS_KEY_ID",
    "COBOT_STATE_R2_SECRET_ACCESS_KEY",
    "COBOT_STATE_PASSPHRASE",
];

/// What a run's Container needs to read and write its `r2://` state.
#[derive(Clone, PartialEq, Eq)]
pub struct StateAccess {
    /// The R2 S3 endpoint.
    pub endpoint: String,
    /// The R2 access key ID.
    pub access_key_id: String,
    /// The R2 secret access key.
    pub secret_access_key: String,
    /// The state's encryption passphrase.
    pub passphrase: String,
}

impl std::fmt::Debug for StateAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateAccess { .. }")
    }
}

/// How a run's Container starts: `serve` in signed mode, for the instance at `root`, against
/// `state`.
#[derive(Clone, PartialEq, Eq)]
pub struct Launch {
    /// The instance's root inside the Container, holding `.config/instance.yaml`.
    pub root: String,
    /// The state every plan and apply uses, e.g. `r2://<bucket>/<prefix>`.
    pub state: String,
    /// Access to that state.
    pub access: StateAccess,
    /// The signing key, `BACKPLANE_TOKEN`.
    pub token: String,
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launch")
            .field("root", &self.root)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Launch {
    /// The Container's command line.
    #[must_use]
    pub fn entrypoint(&self) -> Vec<String> {
        [
            "/usr/local/bin/pigeon",
            "backplane",
            "serve",
            "--listen",
            &format!("0.0.0.0:{PORT}"),
            "--network",
            "--signed",
            "--root",
            &self.root,
            "--state",
            &self.state,
        ]
        .map(str::to_string)
        .to_vec()
    }

    /// The Container's environment: the signing key and the state's access.
    #[must_use]
    pub fn env(&self) -> Vec<(&'static str, &str)> {
        let a = &self.access;
        let [endpoint, key_id, secret, passphrase] = STATE_VARS;
        vec![
            (TOKEN_VAR, self.token.as_str()),
            (endpoint, a.endpoint.as_str()),
            (key_id, a.access_key_id.as_str()),
            (secret, a.secret_access_key.as_str()),
            (passphrase, a.passphrase.as_str()),
        ]
    }
}

/// The variable naming the `pigeon` build a job Container downloads.
pub const BUILD_URL_VAR: &str = "PIGEON_BUILD_URL";
/// The variable holding that build's SHA-256.
pub const BUILD_SHA256_VAR: &str = "PIGEON_BUILD_SHA256";
pub use crate::cobot::ship_wire::{
    SENTINEL_SHA256_VAR, SENTINEL_URL_VAR, TEST_MAP_SHA256_VAR, TEST_MAP_URL_VAR,
};
/// The variable holding the builds token's client ID.
pub const BUILD_ACCESS_ID_VAR: &str = "PIGEON_BUILD_ACCESS_ID";
/// The variable holding the builds token's secret.
pub const BUILD_ACCESS_SECRET_VAR: &str = "PIGEON_BUILD_ACCESS_SECRET";
/// The image's wrapper that runs a named build, else the image's `pigeon`.
pub const START: &str = "/usr/local/bin/pigeon-start";

/// A `pigeon` build a job Container runs: where to download it, and its SHA-256; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §3.
#[derive(Clone, PartialEq, Eq)]
pub struct JobsBuild {
    /// Its URL, behind Access.
    pub url: String,
    /// Its SHA-256, hex.
    pub sha256: String,
    /// The Access service token that may read it: client ID and secret.
    pub access: (String, String),
    /// The `pigeon-sentinel` built beside it: its URL and SHA-256; none for a build without one.
    pub sentinel: Option<(String, String)>,
    /// `main`'s newest test map: its URL and SHA-256; none before the first.
    pub test_map: Option<(String, String)>,
}

impl std::fmt::Debug for JobsBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobsBuild")
            .field("url", &self.url)
            .field("sha256", &self.sha256)
            .field("sentinel", &self.sentinel)
            .field("test_map", &self.test_map)
            .finish_non_exhaustive()
    }
}

/// What a job's Container needs: the signing key, and the build to run if there is one. A
/// job's read token comes with its request, so nothing else lives in the Container's
/// environment.
#[derive(Clone, PartialEq, Eq)]
pub struct JobsLaunch {
    /// The signing key.
    pub token: String,
    /// `main`'s newest `pigeon`, run through [`START`]; none runs the image's own.
    pub build: Option<JobsBuild>,
}

impl std::fmt::Debug for JobsLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobsLaunch").finish_non_exhaustive()
    }
}

impl JobsLaunch {
    /// The Container's command line: `pigeon backplane jobs`, signed; through [`START`] when a
    /// build is named.
    #[must_use]
    pub fn entrypoint(&self) -> Vec<String> {
        let program = if self.build.is_some() {
            START
        } else {
            "/usr/local/bin/pigeon"
        };
        [
            program,
            "backplane",
            "jobs",
            "--listen",
            &format!("0.0.0.0:{PORT}"),
        ]
        .map(str::to_string)
        .to_vec()
    }

    /// The Container's environment: the signing key, and the build's URL and SHA-256.
    #[must_use]
    pub fn env(&self) -> Vec<(&'static str, &str)> {
        let mut env = vec![(TOKEN_VAR, self.token.as_str())];
        if let Some(b) = &self.build {
            env.push((BUILD_URL_VAR, b.url.as_str()));
            env.push((BUILD_SHA256_VAR, b.sha256.as_str()));
            env.push((BUILD_ACCESS_ID_VAR, b.access.0.as_str()));
            env.push((BUILD_ACCESS_SECRET_VAR, b.access.1.as_str()));
            if let Some((url, sha256)) = &b.sentinel {
                env.push((SENTINEL_URL_VAR, url.as_str()));
                env.push((SENTINEL_SHA256_VAR, sha256.as_str()));
            }
            if let Some((url, sha256)) = &b.test_map {
                env.push((TEST_MAP_URL_VAR, url.as_str()));
                env.push((TEST_MAP_SHA256_VAR, sha256.as_str()));
            }
        }
        env
    }
}

/// Why a dispatch failed. An unverified answer is never success.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchError {
    /// The Container did not start.
    Start(String),
    /// The Container never answered, or the request failed in transit.
    Unreachable(String),
    /// The answer's signature is missing or wrong.
    Unverified,
    /// A verified answer whose body is not JSON.
    Malformed(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(e) => write!(f, "the Container did not start: {e}"),
            Self::Unreachable(e) => write!(f, "the Container did not answer: {e}"),
            Self::Unverified => f.write_str(&link::Unverified.to_string()),
            Self::Malformed(e) => write!(f, "the Container's answer is not JSON: {e}"),
        }
    }
}

impl std::error::Error for DispatchError {}

impl From<link::Unverified> for DispatchError {
    fn from(_: link::Unverified) -> Self {
        Self::Unverified
    }
}

/// Checks a signed answer, then parses its body; `nonce` is the request's.
///
/// # Errors
///
/// [`DispatchError::Unverified`] before anything is parsed, and [`DispatchError::Malformed`] for
/// a verified body that is not JSON.
pub fn accept(
    token: &str,
    nonce: &str,
    status: u16,
    body: &[u8],
    signature: Option<&str>,
) -> Result<(u16, serde_json::Value), DispatchError> {
    link::verify_response(token, nonce, status, body, signature)?;
    serde_json::from_slice(body)
        .map(|v| (status, v))
        .map_err(|e| DispatchError::Malformed(e.to_string()))
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{call, monitor, start, start_jobs};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::time::Duration;

    use worker::wasm_bindgen::JsCast;
    use worker::wasm_bindgen::JsValue;
    use worker::{Container, Date, Delay, Fetcher, Headers, Method, RequestInit, State, js_sys};

    use super::{DispatchError, JobsLaunch, Launch, PORT, accept};
    use crate::cobot::link;

    /// Half-second checks of `/healthz` while a cold Container boots: two minutes.
    const READY_TRIES: u32 = 240;

    /// Starts the run's Container unless it is running, and waits until `serve` answers.
    ///
    /// # Errors
    ///
    /// [`DispatchError::Start`] if it does not start, and [`DispatchError::Unreachable`] if
    /// `serve` never answers.
    pub async fn start(
        state: &State,
        container: &Container,
        launch: &Launch,
    ) -> Result<(), DispatchError> {
        boot(state, container, &launch.entrypoint(), &launch.env()).await
    }

    /// As [`start`], for a job's Container.
    ///
    /// # Errors
    ///
    /// As [`start`].
    pub async fn start_jobs(
        state: &State,
        container: &Container,
        launch: &JobsLaunch,
    ) -> Result<(), DispatchError> {
        boot(state, container, &launch.entrypoint(), &launch.env()).await
    }

    /// Starts the Container through the runtime's own `ctx.container.start`, with `env` as a
    /// plain object. workers-rs 0.8.7's `ContainerStartupOptions` passes `env` as a JavaScript
    /// `Map`, which the runtime reads as no variables at all.
    fn start_raw(state: &State, entrypoint: &[String], env: &[(&str, &str)]) -> worker::Result<()> {
        let raw: &JsValue = state.as_raw().as_ref();
        let container = js_sys::Reflect::get(raw, &"container".into())?;
        let start: js_sys::Function =
            js_sys::Reflect::get(&container, &"start".into())?.dyn_into()?;
        let options = js_sys::Object::new();
        let argv = js_sys::Array::new();
        for a in entrypoint {
            argv.push(&JsValue::from_str(a));
        }
        js_sys::Reflect::set(&options, &"entrypoint".into(), &argv)?;
        js_sys::Reflect::set(&options, &"enableInternet".into(), &JsValue::TRUE)?;
        let vars = js_sys::Object::new();
        for (k, v) in env {
            js_sys::Reflect::set(&vars, &JsValue::from_str(k), &JsValue::from_str(v))?;
        }
        js_sys::Reflect::set(&options, &"env".into(), &vars)?;
        start.call1(&container, &options)?;
        Ok(())
    }

    async fn boot(
        state: &State,
        container: &Container,
        entrypoint: &[String],
        env: &[(&str, &str)],
    ) -> Result<(), DispatchError> {
        if !container.running() {
            start_raw(state, entrypoint, env).map_err(|e| DispatchError::Start(e.to_string()))?;
        }
        let port = port(container)?;
        let mut last = String::new();
        for _ in 0..READY_TRIES {
            match port.fetch(url("/healthz"), None).await {
                Ok(r) if r.status_code() == 200 => return Ok(()),
                Ok(r) => last = format!("status {}", r.status_code()),
                Err(e) => last = e.to_string(),
            }
            Delay::from(Duration::from_millis(500)).await;
        }
        Err(DispatchError::Unreachable(last))
    }

    /// Sends one signed request to `serve`, and returns its verified answer.
    ///
    /// # Errors
    ///
    /// [`DispatchError::Unreachable`] in transit, [`DispatchError::Unverified`] for an answer
    /// not signed for this request, and [`DispatchError::Malformed`] for one that is not JSON.
    pub async fn call(
        container: &Container,
        token: &str,
        method: Method,
        path: &str,
        body: &[u8],
    ) -> Result<(u16, serde_json::Value), DispatchError> {
        let unreachable = |e: worker::Error| DispatchError::Unreachable(e.to_string());
        let signed = link::sign(
            token,
            method.as_ref(),
            path,
            Date::now().as_millis() / 1000,
            nonce().map_err(unreachable)?,
            body,
        );
        let headers = Headers::new();
        headers
            .set(link::TIMESTAMP_HEADER, &signed.timestamp.to_string())
            .map_err(unreachable)?;
        headers
            .set(link::NONCE_HEADER, &signed.nonce)
            .map_err(unreachable)?;
        headers
            .set(link::SIGNATURE_HEADER, &signed.signature)
            .map_err(unreachable)?;
        let mut init = RequestInit::new();
        if !body.is_empty() {
            headers
                .set("content-type", "application/json")
                .map_err(unreachable)?;
            init.with_body(Some(js_sys::Uint8Array::from(body).into()));
        }
        init.with_method(method).with_headers(headers);
        let mut response = port(container)?
            .fetch(url(path), Some(init))
            .await
            .map_err(unreachable)?;
        let status = response.status_code();
        let signature = response
            .headers()
            .get(link::SIGNATURE_HEADER)
            .map_err(unreachable)?;
        let bytes = response.bytes().await.map_err(unreachable)?;
        accept(token, &signed.nonce, status, &bytes, signature.as_deref())
    }

    /// Resolves when the run's Container stops: held for a run's whole length, it keeps the
    /// Durable Object alive (ADR §3), and its end before the run's marks the Container lost.
    ///
    /// # Errors
    ///
    /// The runtime's error if the Container exited with one.
    pub async fn monitor(container: &Container) -> worker::Result<()> {
        container.wait_for_exit().await
    }

    fn port(container: &Container) -> Result<Fetcher, DispatchError> {
        container
            .get_tcp_port(PORT)
            .map_err(|e| DispatchError::Unreachable(e.to_string()))
    }

    fn url(path: &str) -> String {
        format!("http://container{path}")
    }

    /// 32 bytes from the runtime's `crypto.getRandomValues`.
    fn nonce() -> worker::Result<[u8; 32]> {
        let crypto = js_sys::Reflect::get(&js_sys::global(), &"crypto".into())?;
        let fill: js_sys::Function =
            js_sys::Reflect::get(&crypto, &"getRandomValues".into())?.dyn_into()?;
        let array = js_sys::Uint8Array::new_with_length(32);
        fill.call1(&crypto, &array)?;
        let mut out = [0u8; 32];
        array.copy_to(&mut out);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatchError, Launch, StateAccess, accept};
    use crate::cobot::link::sign_response;

    const TOKEN: &str = "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk";
    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn launch() -> Launch {
        Launch {
            root: "/instance".into(),
            state: "r2://state/i1".into(),
            access: StateAccess {
                endpoint: "https://r2.example".into(),
                access_key_id: "key-id".into(),
                secret_access_key: "secret".into(),
                passphrase: "passphrase".into(),
            },
            token: TOKEN.into(),
        }
    }

    #[test]
    fn a_run_launches_serve_signed_for_its_instance_and_state() {
        let launch = launch();
        assert_eq!(
            launch.entrypoint(),
            [
                "/usr/local/bin/pigeon",
                "backplane",
                "serve",
                "--listen",
                "0.0.0.0:8080",
                "--network",
                "--signed",
                "--root",
                "/instance",
                "--state",
                "r2://state/i1",
            ]
        );
        assert_eq!(
            launch.env(),
            [
                ("BACKPLANE_TOKEN", TOKEN),
                ("COBOT_STATE_R2_ENDPOINT", "https://r2.example"),
                ("COBOT_STATE_R2_ACCESS_KEY_ID", "key-id"),
                ("COBOT_STATE_R2_SECRET_ACCESS_KEY", "secret"),
                ("COBOT_STATE_PASSPHRASE", "passphrase"),
            ]
        );
    }

    #[test]
    fn a_job_starts_through_pigeon_start_only_when_a_build_is_named() {
        let mut jobs = super::JobsLaunch {
            token: TOKEN.into(),
            build: None,
        };
        assert_eq!(jobs.entrypoint()[0], "/usr/local/bin/pigeon");
        assert_eq!(jobs.env(), [("BACKPLANE_TOKEN", TOKEN)]);
        let sha256 = "ab".repeat(32);
        jobs.build = Some(super::JobsBuild {
            url: "https://control.example/builds/x/pigeon".into(),
            sha256: sha256.clone(),
            access: ("builds-id".into(), "builds-secret".into()),
            sentinel: None,
            test_map: None,
        });
        assert!(!format!("{jobs:?}").contains("builds-secret"));
        assert!(!format!("{:?}", jobs.build.as_ref().expect("a build")).contains("builds-secret"));
        assert_eq!(
            jobs.entrypoint(),
            [
                "/usr/local/bin/pigeon-start",
                "backplane",
                "jobs",
                "--listen",
                "0.0.0.0:8080"
            ]
        );
        assert_eq!(
            jobs.env(),
            [
                ("BACKPLANE_TOKEN", TOKEN),
                (
                    "PIGEON_BUILD_URL",
                    "https://control.example/builds/x/pigeon"
                ),
                ("PIGEON_BUILD_SHA256", sha256.as_str()),
                ("PIGEON_BUILD_ACCESS_ID", "builds-id"),
                ("PIGEON_BUILD_ACCESS_SECRET", "builds-secret"),
            ]
        );
        let sentinel = "cd".repeat(32);
        if let Some(b) = jobs.build.as_mut() {
            b.sentinel = Some((
                "https://control.example/builds/x/pigeon-sentinel".into(),
                sentinel.clone(),
            ));
        }
        assert_eq!(
            jobs.env()[5..],
            [
                (
                    "PIGEON_SENTINEL_URL",
                    "https://control.example/builds/x/pigeon-sentinel"
                ),
                ("PIGEON_SENTINEL_SHA256", sentinel.as_str()),
            ]
        );
        let map = "ef".repeat(32);
        if let Some(b) = jobs.build.as_mut() {
            b.test_map = Some((
                "https://control.example/builds/x/tests.json".into(),
                map.clone(),
            ));
        }
        assert_eq!(
            jobs.env()[7..],
            [
                (
                    "PIGEON_TEST_MAP_URL",
                    "https://control.example/builds/x/tests.json"
                ),
                ("PIGEON_TEST_MAP_SHA256", map.as_str()),
            ]
        );
    }

    #[test]
    fn a_build_path_names_pigeon_or_pigeon_sentinel_and_nothing_else() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            crate::cobot::ship_wire::build_path(&format!("/builds/{sha}/pigeon-sentinel")),
            Some((sha, "pigeon-sentinel"))
        );
        assert_eq!(
            crate::cobot::ship_wire::build_path(&format!("/builds/{sha}/pigeon")),
            Some((sha, "pigeon"))
        );
        assert_eq!(
            crate::cobot::ship_wire::build_path(&format!("/builds/{sha}/tests.json")),
            Some((sha, "tests.json"))
        );
        assert_eq!(
            crate::cobot::ship_wire::build_path_sha(&format!("/builds/{sha}/pigeon-sentinel")),
            None
        );
        assert_eq!(
            crate::cobot::ship_wire::sentinel_key(sha),
            format!("builds/{sha}/pigeon-sentinel")
        );
        for bad in [
            format!("/builds/{sha}/pigeon-other"),
            format!("/builds/{sha}/pigeon-sentinel/x"),
            format!("/builds/{sha}/"),
        ] {
            assert_eq!(crate::cobot::ship_wire::build_path(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_build_path_names_one_full_commit_and_nothing_else() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            crate::cobot::ship_wire::build_path_sha(&format!("/builds/{sha}/pigeon")),
            Some(sha)
        );
        assert_eq!(
            crate::cobot::ship_wire::build_key(sha),
            format!("builds/{sha}/pigeon")
        );
        for bad in [
            "/builds/0123456789ab/pigeon".to_string(),
            format!("/builds/{}/pigeon", sha.to_uppercase()),
            format!("/builds/{sha}/other"),
            format!("/builds/{sha}/../ci/log/pigeon"),
            format!("/builds/../{sha}/pigeon"),
            format!("/ci/{sha}/pigeon"),
        ] {
            assert_eq!(crate::cobot::ship_wire::build_path_sha(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_launch_never_prints_a_secret() {
        let shown = format!("{:?}", launch());
        for secret in [TOKEN, "key-id", "secret", "passphrase"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }

    #[cfg(feature = "engines")]
    #[test]
    fn the_state_variables_are_cobots() {
        assert_eq!(
            super::STATE_VARS,
            [
                crate::cobot::state::ENDPOINT_VAR,
                crate::cobot::state::ACCESS_KEY_VAR,
                crate::cobot::state::SECRET_KEY_VAR,
                crate::cobot::PASSPHRASE_VAR,
            ]
        );
    }

    #[test]
    fn only_a_signed_answer_is_read() {
        let body = br#"{"ok":true}"#;
        let sig = sign_response(TOKEN, NONCE, 200, body);
        assert_eq!(
            accept(TOKEN, NONCE, 200, body, Some(&sig)),
            Ok((200, serde_json::json!({ "ok": true })))
        );
        assert_eq!(
            accept(TOKEN, NONCE, 200, body, None),
            Err(DispatchError::Unverified)
        );
        assert_eq!(
            accept(TOKEN, NONCE, 409, body, Some(&sig)),
            Err(DispatchError::Unverified),
            "a changed status"
        );
        let bad = sign_response(TOKEN, NONCE, 200, b"not json");
        assert!(matches!(
            accept(TOKEN, NONCE, 200, b"not json", Some(&bad)),
            Err(DispatchError::Malformed(_))
        ));
    }
}
