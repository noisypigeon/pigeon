//! A Worker's container images built and pushed from a job Container, with rootless `BuildKit`
//! fetched pinned by SHA-256; see
//! `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §4 and its amendments.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::Error;
use crate::workers::{self, Found, Image, Tools};

/// `BuildKit`'s release.
pub const BUILDKIT: &str = "v0.26.2";
/// Its `linux-amd64` tarball's SHA-256, measured when pinned; the release publishes none.
pub const BUILDKIT_SHA256: &str =
    "1ef7c888f808e7f3f49d9aeeca11f661afe5c0880a4b114cc31c56dee86acd35";
/// `RootlessKit`'s release.
pub const ROOTLESSKIT: &str = "v2.3.5";
/// Its `x86_64` tarball's SHA-256, as its `SHA256SUMS` publishes.
pub const ROOTLESSKIT_SHA256: &str =
    "118208e25becd144ee7317c172fc9decce7b16174d5c1bbf80f1d1d0eacc6b5f";

/// How long `buildkitd` has to answer once started.
const READY: Duration = Duration::from_secs(30);

/// The programs a build runs.
#[derive(Debug, Clone)]
pub struct Builder {
    /// `rootlesskit`.
    pub rootlesskit: PathBuf,
    /// `buildkitd`.
    pub buildkitd: PathBuf,
    /// `buildctl`.
    pub buildctl: PathBuf,
}

fn tool(command: &str, message: impl Into<String>) -> Error {
    Error::Tool {
        command: command.into(),
        message: message.into(),
    }
}

/// Whether `file`'s SHA-256 is `want`.
#[must_use]
pub fn verified(file: &Path, want: &str) -> bool {
    fs::read(file).is_ok_and(|b| crate::s3::sha256_hex(&b) == want)
}

/// Downloads `BuildKit` and `RootlessKit` into `work/bin` with `curl`, refusing a tarball whose
/// SHA-256 differs from its pin, and unpacks them with `tar`.
///
/// # Errors
///
/// [`Error::Tool`] for a failed download, a checksum that differs, or a failed unpack.
pub fn fetch(work: &Path) -> Result<Builder, Error> {
    let bin = work.join("bin");
    fs::create_dir_all(&bin).map_err(|e| tool("image tools", e.to_string()))?;
    let get = |url: &str, sha: &str, name: &str| -> Result<PathBuf, Error> {
        let out = work.join(name);
        let ok = Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&out)
            .arg(url)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err(tool("image tools", format!("{url} did not download")));
        }
        if !verified(&out, sha) {
            return Err(tool(
                "image tools",
                format!("{url}'s SHA-256 differs from its pin"),
            ));
        }
        Ok(out)
    };
    let bk = get(
        &format!(
            "https://github.com/moby/buildkit/releases/download/{BUILDKIT}/buildkit-{BUILDKIT}.linux-amd64.tar.gz"
        ),
        BUILDKIT_SHA256,
        "buildkit.tgz",
    )?;
    let rk = get(
        &format!(
            "https://github.com/rootless-containers/rootlesskit/releases/download/{ROOTLESSKIT}/rootlesskit-x86_64.tar.gz"
        ),
        ROOTLESSKIT_SHA256,
        "rootlesskit.tgz",
    )?;
    let untar = |file: &Path, into: &Path, members: &[&str]| -> Result<(), Error> {
        let ok = Command::new("tar")
            .arg("-xzf")
            .arg(file)
            .arg("-C")
            .arg(into)
            .args(members)
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            Ok(())
        } else {
            Err(tool(
                "image tools",
                format!("{} did not unpack", file.display()),
            ))
        }
    };
    untar(
        &bk,
        work,
        &["bin/buildkitd", "bin/buildctl", "bin/buildkit-runc"],
    )?;
    untar(&rk, &bin, &["rootlesskit"])?;
    let _ = fs::remove_file(bk);
    let _ = fs::remove_file(rk);
    Ok(Builder {
        rootlesskit: bin.join("rootlesskit"),
        buildkitd: bin.join("buildkitd"),
        buildctl: bin.join("buildctl"),
    })
}

/// Base64, as a Docker config's `auth` holds `user:password`.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `buildkitd` under `RootlessKit`, stopped when dropped.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The images `found`'s containers need, built from the checkout at `root` and pushed to
/// Cloudflare's registry as `<worker>-<class>:<tag>`: each distinct image is built once and
/// pushed under every class that uses it. The registry's credentials last `minutes`, reach
/// `BuildKit` only through a Docker config in `work`, and are scrubbed from every output.
///
/// # Errors
///
/// [`Error::Tool`] when the credentials, the daemon or a build fail.
#[allow(clippy::too_many_arguments)]
pub fn build_and_push(
    root: &Path,
    found: &Found,
    tag: &str,
    account: &str,
    api: &crate::cloudflare::Api,
    minutes: u32,
    builder: &Builder,
    tools: &Tools,
    work: &Path,
    log: &mut String,
) -> Result<BTreeMap<String, String>, Error> {
    let (user, password) = workers::registry_credentials(api, account, minutes)
        .map_err(|m| tool("registry credentials", m))?;
    let mut tools = tools.clone();
    tools.secrets.push(password.clone());
    let auth = base64(format!("{user}:{password}").as_bytes());
    tools.secrets.push(auth.clone());
    let config = work.join("docker");
    fs::create_dir_all(&config).map_err(|e| tool("image build", e.to_string()))?;
    fs::write(
        config.join("config.json"),
        serde_json::json!({ "auths": { workers::REGISTRY: { "auth": auth } } }).to_string(),
    )
    .map_err(|e| tool("image build", e.to_string()))?;
    let run = work.join("run");
    fs::create_dir_all(&run).map_err(|e| tool("image build", e.to_string()))?;
    let user_name = std::env::var("USER").unwrap_or_else(|_| "nonroot".into());
    let env = [
        ("USER".to_string(), user_name),
        ("HOME".to_string(), work.to_string_lossy().into_owned()),
        (
            "XDG_RUNTIME_DIR".to_string(),
            run.to_string_lossy().into_owned(),
        ),
        (
            "DOCKER_CONFIG".to_string(),
            config.to_string_lossy().into_owned(),
        ),
    ];
    tools.env.extend(env.iter().cloned());
    let (daemon, addr, sockdir) = start_daemon(root, builder, &tools, work, &env)?;
    let mut groups: BTreeMap<String, (Image, Vec<String>)> = BTreeMap::new();
    for c in &found.spec.containers {
        let key = serde_json::to_string(&c.image).unwrap_or_default();
        groups
            .entry(key)
            .or_insert_with(|| (c.image.clone(), Vec::new()))
            .1
            .push(c.class.clone());
    }
    let mut out = BTreeMap::new();
    for (image, classes) in groups.into_values() {
        let names: Vec<String> = classes
            .iter()
            .map(|class| {
                format!(
                    "{}/{account}/{}-{}:{tag}",
                    workers::REGISTRY,
                    found.spec.name,
                    class.to_ascii_lowercase()
                )
            })
            .collect();
        let args = build_args(root, &addr, &image, &names);
        let started = std::time::Instant::now();
        tools.run(&builder.buildctl, &args, root, None)?;
        let _ = writeln!(
            log,
            "built and pushed {} ({}) in {}s",
            classes.join(", "),
            image.target,
            started.elapsed().as_secs()
        );
        for (class, name) in classes.iter().zip(names) {
            out.insert(workers::image_key(&found.spec, class), name);
        }
    }
    drop(daemon);
    let _ = fs::remove_dir_all(&sockdir);
    let _ = fs::remove_file(config.join("config.json"));
    Ok(out)
}

/// `PATH` with `program`'s directory first, so `RootlessKit` finds `buildkit-runc` beside it.
fn path_with(program: &Path) -> String {
    let dir = program
        .parent()
        .map_or_else(String::new, |d| d.to_string_lossy().into_owned());
    match std::env::var("PATH") {
        Ok(p) if !p.is_empty() => format!("{dir}:{p}"),
        _ => dir,
    }
}

/// Starts `buildkitd` under `RootlessKit` with `env`, and waits until it answers: the daemon, its
/// address, and the short directory holding its socket.
fn start_daemon(
    root: &Path,
    builder: &Builder,
    tools: &Tools,
    work: &Path,
    env: &[(String, String)],
) -> Result<(Daemon, String, PathBuf), Error> {
    // A socket path must stay under 104 bytes, so it lives in a short directory of its own.
    let sockdir = std::env::temp_dir().join(format!("bk-{}", std::process::id()));
    fs::create_dir_all(&sockdir).map_err(|e| tool("image build", e.to_string()))?;
    let addr = format!("unix://{}/buildkitd.sock", sockdir.display());
    let daemon_log = fs::File::create(work.join("buildkitd.log"))
        .map_err(|e| tool("image build", e.to_string()))?;
    let daemon = Command::new(&builder.rootlesskit)
        .arg(format!(
            "--state-dir={}",
            work.join("rootlesskit").display()
        ))
        .args([
            "--net=host",
            "--copy-up=/etc",
            "--copy-up=/run",
            "--disable-host-loopback",
        ])
        .arg(&builder.buildkitd)
        .args(["--rootless", "--addr", &addr, "--root"])
        .arg(work.join("root"))
        .args(["--oci-worker-no-process-sandbox"])
        .envs(env.iter().map(|(k, v)| (k, v)))
        .env("PATH", path_with(&builder.buildkitd))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(daemon_log)
        .spawn()
        .map_err(|e| tool("buildkitd", e.to_string()))?;
    let daemon = Daemon(daemon);
    let ready = (0..READY.as_secs()).any(|_| {
        let ok = tools
            .run(
                &builder.buildctl,
                &[
                    "--addr".into(),
                    addr.clone(),
                    "debug".into(),
                    "workers".into(),
                ],
                root,
                None,
            )
            .is_ok();
        if !ok {
            sleep(Duration::from_secs(1));
        }
        ok
    });
    if !ready {
        let said = fs::read_to_string(work.join("buildkitd.log")).unwrap_or_default();
        let last: Vec<&str> = said.lines().rev().take(5).collect();
        return Err(tool(
            "buildkitd",
            format!(
                "did not answer: {}",
                last.into_iter().rev().collect::<Vec<_>>().join(" | ")
            ),
        ));
    }
    Ok((daemon, addr, sockdir))
}

/// `buildctl build`'s arguments for `image`, pushed as `names`.
fn build_args(root: &Path, addr: &str, image: &Image, names: &[String]) -> Vec<String> {
    let file = root.join(&image.dockerfile);
    let context = file.parent().unwrap_or(root).to_path_buf();
    let mut args: Vec<String> = vec![
        "--addr".into(),
        addr.to_string(),
        "build".into(),
        "--progress".into(),
        "plain".into(),
        "--frontend".into(),
        "dockerfile.v0".into(),
        "--local".into(),
        format!("context={}", context.display()),
        "--local".into(),
        format!("dockerfile={}", context.display()),
        "--opt".into(),
        format!(
            "filename={}",
            file.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ),
        "--opt".into(),
        format!("target={}", image.target),
        "--opt".into(),
        format!("platform={}", workers::PLATFORM),
    ];
    for (k, v) in &image.args {
        args.extend(["--opt".into(), format!("build-arg:{k}={v}")]);
    }
    args.extend([
        "--output".into(),
        format!("type=image,\"name={}\",push=true", names.join(",")),
    ]);
    args
}
