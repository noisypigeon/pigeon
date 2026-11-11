//! `pigeon backplane signing`, the shell around `backplane::signing`; see
//! `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md` §6.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use backplane::push::{b64url, b64url_decode};
use backplane::signing::{
    Grant, Layout, ROUTE, SECURITY, State, Step, check_step, clean_plan, load_plan, oidc_argv,
    parse_search_list, upload_argv,
};

use super::{INVALID, SUCCESS, VIOLATION};
use crate::cli::{SigningArgs, SigningCommand, UploadArgs};

/// Runs a `pigeon backplane signing` command and returns its exit code.
#[must_use]
pub fn run(command: SigningCommand) -> i32 {
    let result = match command {
        SigningCommand::Load(args) => load(&args),
        SigningCommand::Upload(args) => upload(&args),
        SigningCommand::Clean(_) => clean(),
    };
    match result {
        Ok(()) => SUCCESS,
        Err(Failure::Refused(e)) => {
            eprintln!("signing: {e}");
            VIOLATION
        }
        Err(Failure::Broken(e)) => {
            eprintln!("signing: {e}");
            INVALID
        }
    }
}

enum Failure {
    /// The step or the control plane refused.
    Refused(String),
    /// Something on this machine failed.
    Broken(String),
}

fn broken(e: impl std::fmt::Display) -> Failure {
    Failure::Broken(e.to_string())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn tmp() -> PathBuf {
    env("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

fn job_dir() -> Result<PathBuf, Failure> {
    let job = env("BUILDKITE_JOB_ID").unwrap_or_default();
    Layout::dir_for(&tmp(), &job).map_err(Failure::Refused)
}

fn load(args: &SigningArgs) -> Result<(), Failure> {
    check_step(env("BUILDKITE_STEP_KEY").as_deref()).map_err(Failure::Refused)?;
    let job = env("BUILDKITE_JOB_ID").unwrap_or_default();
    let home = env("HOME").ok_or_else(|| broken("HOME is unset"))?;
    if job_dir()?.join("state.json").is_file() {
        clean()?;
    }

    let token =
        output(&oidc_argv()).map_err(|e| Failure::Refused(format!("no OIDC token: {e}")))?;
    let grant = exchange(&args.platform, token.trim())?;
    grant.validate().map_err(Failure::Refused)?;

    let layout = Layout::new(&tmp(), Path::new(&home), &job, &grant.profile_uuid)
        .map_err(Failure::Refused)?;
    let listed =
        output(&[SECURITY, "list-keychains", "-d", "user"].map(String::from)).map_err(broken)?;
    let state = State {
        layout,
        search_list: parse_search_list(&listed),
    };
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&state.layout.dir)
        .map_err(|e| broken(format!("{}: {e}", state.layout.dir.display())))?;
    let json = serde_json::to_vec(&state).map_err(broken)?;
    write(&state.layout.state(), &json)?;

    let p12 =
        b64url_decode(&grant.p12).map_err(|e| Failure::Refused(format!("the grant's p12: {e}")))?;
    let profile = b64url_decode(&grant.profile)
        .map_err(|e| Failure::Refused(format!("the grant's profile: {e}")))?;
    let plan = load_plan(&grant, &state, p12, profile, &random()?, &random()?);
    for step in plan {
        if let Err(e) = apply(&step) {
            let _ = clean();
            return Err(e);
        }
    }
    println!("{}", state.layout.export_options().display());
    eprintln!(
        "signing: loaded; the upload token expires at {}",
        grant.expires
    );
    Ok(())
}

fn exchange(platform: &str, token: &str) -> Result<Grant, Failure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let url = format!("{}{ROUTE}", platform.trim_end_matches('/'));
    let mut response = agent
        .post(&url)
        .header("authorization", &format!("Bearer {token}"))
        .send_empty()
        .map_err(|e| Failure::Refused(format!("POST {ROUTE}: {e}")))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(Failure::Refused(format!(
            "POST {ROUTE}: refused with {status}"
        )));
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| Failure::Refused(format!("POST {ROUTE}: {e}")))?;
    serde_json::from_str(&body)
        .map_err(|_| Failure::Refused(format!("POST {ROUTE}: the answer isn't a grant")))
}

fn upload(args: &UploadArgs) -> Result<(), Failure> {
    let dir = job_dir()?;
    let state: State =
        read_state(&dir)?.ok_or_else(|| broken("nothing is loaded; run `load` first"))?;
    let jwt = fs::read_to_string(state.layout.jwt()).map_err(broken)?;
    let status = Command::new(&upload_argv(&args.ipa, jwt.trim())[0])
        .args(&upload_argv(&args.ipa, jwt.trim())[1..])
        .status()
        .map_err(broken)?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::Refused(format!("the upload failed: {status}")))
    }
}

fn clean() -> Result<(), Failure> {
    let dir = job_dir()?;
    let Some(state) = read_state(&dir)? else {
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(broken)?;
        }
        return Ok(());
    };
    let mut failed = 0;
    for step in clean_plan(&state) {
        if let Err(Failure::Broken(e) | Failure::Refused(e)) = apply(&step) {
            eprintln!("signing: clean: {e}");
            failed += 1;
        }
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(broken(format!("{failed} clean step(s) failed")))
    }
}

fn read_state(dir: &Path) -> Result<Option<State>, Failure> {
    match fs::read(dir.join("state.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(broken),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(broken(e)),
    }
}

fn apply(step: &Step) -> Result<(), Failure> {
    match step {
        Step::Write(path, secret) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(broken)?;
            }
            write(path, &secret.0)
        }
        Step::Run(cmd) => {
            let status = Command::new(&cmd.argv[0])
                .args(&cmd.argv[1..])
                .envs(cmd.env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .status()
                .map_err(|e| broken(format!("{}: {e}", cmd.argv[0])))?;
            if status.success() {
                Ok(())
            } else {
                let what = cmd.argv.get(1).map_or("", String::as_str);
                Err(broken(format!("{} {what}: {status}", cmd.argv[0])))
            }
        }
        Step::Remove(path) => match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(broken(e)),
            _ => Ok(()),
        },
        Step::RemoveDir(path) => match fs::remove_dir_all(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(broken(e)),
            _ => Ok(()),
        },
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| broken(format!("{}: {e}", path.display())))
}

fn output(argv: &[String]) -> Result<String, String> {
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if out.status.success() {
        String::from_utf8(out.stdout).map_err(|e| e.to_string())
    } else {
        Err(format!("{} exited {}", argv[0], out.status))
    }
}

/// 24 random bytes, base64url: a one-time password.
fn random() -> Result<String, Failure> {
    let mut bytes = [0u8; 24];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| broken(format!("no randomness: {e}")))?;
    Ok(b64url(&bytes))
}
