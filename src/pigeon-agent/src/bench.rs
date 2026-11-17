//! A bench's job server, in the shell: one session's long-lived checkout, synced from its
//! scratch ref, and its allowed commands run with no network as uid 1000; see
//! `.claude/decisions/2026-11-16-development-session-bench-containers.md` §3.2, §5 and its
//! 2026-11-16 amendments.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write as _};
use std::path::PathBuf;
use std::process::{Command as Proc, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::bench::{self, allow::Command, deal::Check};

/// The uid a command runs as inside the namespace (spike amendment).
pub const UID: &str = "1000";

/// The server's environment a command keeps; everything else is cleared.
pub const KEPT: [&str; 6] = [
    "PATH",
    "RUSTUP_HOME",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_PROFILE_DEV_DEBUG",
    "CARGO_PROFILE_TEST_DEBUG",
];

/// git with no hooks and no fsmonitor, so nothing a command left in `.git` runs later.
pub const GIT_SAFE: [&str; 5] = [
    "git",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "core.fsmonitor=false",
];

/// How often a running step is checked for its end, a cancel or its timeout.
const TICK: Duration = Duration::from_millis(200);

/// What a command runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// New user, PID and mount namespaces through `unshare --map-auto`, a private `/tmp`, then
    /// uid 1000 through `setpriv`; a new network namespace, loopback only, unless the step
    /// fetches.
    Unshare,
    /// The command as it is: for tests, where user namespaces may be refused.
    Plain,
}

/// Whether a step may reach the network, and as whom it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// uid 1000, no network: every allowed command.
    Command,
    /// uid 1000, with the network: the server's git fetch and `cargo fetch`.
    Fetch,
    /// The namespace's root, no network: the first `chown` to uid 1000.
    Own,
}

impl Sandbox {
    /// `argv` wrapped for `mode`.
    fn wrap(self, argv: &[String], mode: Mode) -> Vec<String> {
        if self == Sandbox::Plain {
            return argv.to_vec();
        }
        let mut out: Vec<String> = ["unshare", "--user", "--map-root-user", "--map-auto"]
            .map(String::from)
            .to_vec();
        if mode != Mode::Fetch {
            out.push("--net".into());
        }
        out.extend(
            [
                "--pid",
                "--fork",
                "--kill-child",
                "--mount-proc",
                "--",
                "sh",
                "-c",
            ]
            .map(String::from),
        );
        // Tests serve on 127.0.0.1, which a new network namespace has down.
        let lo = if mode == Mode::Fetch {
            ""
        } else {
            "ip link set lo up || exit 125; "
        };
        let inner = if mode == Mode::Own {
            "exec \"$@\"".to_string()
        } else {
            format!(
                "{lo}mount -t tmpfs -o mode=1777 tmpfs /tmp || exit 125; \
                 exec setpriv --reuid={UID} --regid={UID} --clear-groups -- \"$@\""
            )
        };
        out.push(inner);
        out.push("sh".into());
        out.extend(argv.iter().cloned());
        out
    }
}

/// A run's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Fetching and checking out its commit.
    Syncing,
    /// Running its command.
    Running,
    /// Its command exited 0.
    Passed,
    /// Its sync or command failed.
    Failed,
    /// Killed at its timeout.
    TimedOut,
    /// Cancelled.
    Cancelled,
}

impl State {
    /// Whether the run has ended.
    #[must_use]
    pub fn done(self) -> bool {
        !matches!(self, State::Syncing | State::Running)
    }
}

/// What a run reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    /// Its ID.
    pub run: String,
    /// Its state.
    pub state: State,
    /// Its command's exit code, once it ended by itself.
    pub exit: Option<i32>,
    /// Seconds since it started, or its whole time once ended.
    pub seconds: u64,
    /// The commit it runs.
    pub sha: String,
    /// Whether the disk guard cleaned the workspace's crates first.
    pub cleaned: bool,
    /// The target directory's size, in MB, when it ended.
    pub target_mb: Option<u64>,
    /// Free disk, in MB, when it ended.
    pub free_mb: Option<u64>,
}

/// A run asked for: its commit, from its session's scratch ref, and its command.
#[derive(Clone, Deserialize)]
pub struct RunSpec {
    /// Its ID: letters, digits and `-`.
    pub run: String,
    /// The repository's git remote.
    pub remote: String,
    /// A read token minted for this run; never logged.
    pub token: String,
    /// The session's scratch ref.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The commit to run: the scratch ref's head.
    pub sha: String,
    /// The default branch, fetched as `artifacts/<base>`.
    #[serde(default = "default_base")]
    pub base: String,
    /// The allowed command.
    pub command: Command,
    /// Its timeout, in minutes.
    #[serde(default)]
    pub timeout: Option<u32>,
}

fn default_base() -> String {
    "main".into()
}

/// Shows no token.
impl std::fmt::Debug for RunSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSpec")
            .field("run", &self.run)
            .field("reference", &self.reference)
            .field("sha", &self.sha)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// One step of a command: its directory under the checkout and its argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Relative to the checkout's root.
    pub dir: String,
    /// The program and its arguments.
    pub argv: Vec<String>,
}

/// The steps of task `name`: through `mise_toml` while the commit has one, else one step running
/// the commit's own `pigeon dev <name>` from its root (bench ADR's step-7 amendment).
///
/// # Errors
///
/// As [`mise_steps`], or a task the allowlist doesn't name.
pub fn task_steps(mise_toml: Option<&str>, name: &str) -> Result<Vec<Step>, String> {
    if let Some(text) = mise_toml {
        return mise_steps(text, name);
    }
    if !bench::allow::TASKS.contains(&name) {
        return Err(format!("bench: task `{name}` isn't allowed"));
    }
    let argv = [
        "cargo", "run", "-q", "--locked", "--bin", "pigeon", "--", "dev", name,
    ];
    Ok(vec![Step {
        dir: String::new(),
        argv: argv.map(String::from).to_vec(),
    }])
}

/// The steps of mise task `name` in `mise_toml`: its `depends` depth first, each once, then its
/// `run`, each line under `sh -c` in its `dir`; every task reached must be allowed.
///
/// # Errors
///
/// When the file doesn't parse, a task is missing or not allowed, or the tasks loop.
pub fn mise_steps(mise_toml: &str, name: &str) -> Result<Vec<Step>, String> {
    let doc: toml::Value = mise_toml
        .parse()
        .map_err(|e| format!("bench: mise.toml doesn't parse: {e}"))?;
    let tasks = doc
        .get("tasks")
        .and_then(toml::Value::as_table)
        .ok_or("bench: mise.toml has no tasks")?;
    let mut out = Vec::new();
    let mut done = Vec::new();
    let mut path = Vec::new();
    visit(tasks, name, &mut path, &mut done, &mut out)?;
    Ok(out)
}

fn visit(
    tasks: &toml::value::Table,
    name: &str,
    path: &mut Vec<String>,
    done: &mut Vec<String>,
    out: &mut Vec<Step>,
) -> Result<(), String> {
    if done.iter().any(|d| d == name) {
        return Ok(());
    }
    if path.iter().any(|p| p == name) {
        return Err(format!("bench: mise task `{name}` depends on itself"));
    }
    if !bench::allow::TASKS.contains(&name) {
        return Err(match path.last() {
            Some(parent) => {
                format!("bench: mise task `{parent}` depends on `{name}`, which isn't allowed")
            }
            None => format!("bench: mise task `{name}` isn't allowed"),
        });
    }
    let task = tasks
        .get(name)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("bench: mise.toml has no task `{name}`"))?;
    path.push(name.to_string());
    let depends: Vec<&str> = match task.get("depends") {
        Some(toml::Value::Array(a)) => a.iter().filter_map(toml::Value::as_str).collect(),
        Some(toml::Value::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    };
    for d in depends {
        visit(tasks, d, path, done, out)?;
    }
    path.pop();
    let dir = task
        .get("dir")
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .trim_matches('/')
        .to_string();
    if dir.split('/').any(|c| c == "..") {
        return Err(format!(
            "bench: mise task `{name}`'s dir leaves the checkout"
        ));
    }
    let runs: Vec<&str> = match task.get("run") {
        Some(toml::Value::String(s)) => vec![s.as_str()],
        Some(toml::Value::Array(a)) => a.iter().filter_map(toml::Value::as_str).collect(),
        _ => Vec::new(),
    };
    for run in runs {
        out.push(Step {
            dir: dir.clone(),
            argv: vec!["sh".into(), "-c".into(), run.to_string()],
        });
    }
    done.push(name.to_string());
    Ok(())
}

/// The argv a bench's `bench check` reads the change's checks with, from the workspace at `root`.
fn scope_argv(root: &str) -> Vec<String> {
    [
        "cargo",
        "run",
        "-q",
        "--locked",
        "-p",
        "pigeon-harness",
        "--",
        "dev",
        "scope",
        "--root",
        root,
        "--base",
        "artifacts/main",
        "--json",
    ]
    .map(String::from)
    .to_vec()
}

/// The check named `name` among scope's `--json` answer.
///
/// # Errors
///
/// When the answer doesn't parse, or names no such check.
pub fn check_step(scope_json: &str, name: &str, dir: &str) -> Result<Step, String> {
    let checks: Vec<Check> = serde_json::from_str(scope_json)
        .map_err(|e| format!("bench: scope's answer doesn't parse: {e}"))?;
    checks
        .into_iter()
        .find(|c| c.name == name)
        .map(|c| Step {
            dir: dir.into(),
            argv: c.argv,
        })
        .ok_or_else(|| format!("bench: no check `{name}` in this change's scope"))
}

/// Where an allowed `argv` runs: `cargo` in the workspace `ws`, anything else (`pigeon`, `git`)
/// at the checkout's root, which their own `--root` defaults to.
fn argv_dir(argv: &[String], ws: &str) -> String {
    if argv.first().is_some_and(|p| p == "cargo") {
        ws.into()
    } else {
        String::new()
    }
}

/// The checkout's root, relative to its workspace directory `ws`.
fn root_from(ws: &str) -> String {
    if ws.is_empty() {
        ".".into()
    } else {
        vec![".."; ws.split('/').count()].join("/")
    }
}

/// The longest prefix of `bytes` that is whole UTF-8.
#[must_use]
pub fn whole_utf8(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(e) if e.error_len().is_none() => &bytes[..e.valid_up_to()],
        _ => bytes,
    }
}

#[derive(Debug, Default)]
struct Inner {
    runs: BTreeMap<String, (RunInfo, Arc<AtomicBool>, Instant)>,
    current: Option<String>,
    owned: bool,
    lock: Option<u64>,
    sha: Option<String>,
}

/// A bench: its checkout under `work`, its runs, and how commands run.
#[derive(Debug)]
pub struct Bench {
    work: PathBuf,
    sandbox: Sandbox,
    env: Vec<(String, String)>,
    inner: Mutex<Inner>,
}

/// Removes the `*.lock` files a git killed by a cancel or timeout left in `git_dir`'s top level
/// and under `refs/`; runs are one at a time, so none is live.
fn clear_locks(git_dir: &std::path::Path) {
    fn walk(dir: &std::path::Path, deep: bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && deep {
                walk(&path, true);
            } else if !is_dir && path.extension().is_some_and(|x| x == "lock") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    walk(git_dir, false);
    walk(&git_dir.join("refs"), true);
}

/// `pigeon-start`'s judge build copied under `work`, out of the `/tmp` the sandbox hides; its
/// path, or none if it can't be copied.
fn keep_judge(from: &str, work: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = work.join("bin");
    let to = dir.join(crate::eviction::BINARY);
    let kept = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::copy(from, &to))
        .and_then(|_| std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755)));
    match kept {
        Ok(()) => Some(to.to_string_lossy().into_owned()),
        Err(e) => {
            eprintln!("bench: the judge's build at {from} was not kept: {e}");
            None
        }
    }
}

impl Bench {
    /// A bench checking out under `work`, keeping [`KEPT`] from `var`.
    #[must_use]
    pub fn new(work: PathBuf, sandbox: Sandbox, var: impl Fn(&str) -> Option<String>) -> Self {
        let mut env: Vec<(String, String)> = KEPT
            .iter()
            .filter_map(|k| var(k).map(|v| ((*k).to_string(), v)))
            .collect();
        if let Some(p) = var(crate::eviction::VAR).and_then(|from| keep_judge(&from, &work)) {
            env.push((crate::eviction::VAR.into(), p));
        }
        Self {
            work,
            sandbox,
            env,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn checkout(&self) -> PathBuf {
        self.work.join("pigeon")
    }

    /// The checkout's Rust workspace, from its root: empty once the workspace is the root.
    fn workspace(&self) -> String {
        crate::checks::scope::workspace_dir(&self.checkout())
    }

    fn log_path(&self, run: &str) -> PathBuf {
        self.work.join("runs").join(format!("{run}.log"))
    }

    fn target(&self) -> PathBuf {
        self.env
            .iter()
            .find(|(k, _)| k == "CARGO_TARGET_DIR")
            .map_or_else(
                || self.checkout().join(self.workspace()).join("target"),
                |(_, v)| PathBuf::from(v),
            )
    }

    fn cargo_home(&self) -> Option<PathBuf> {
        self.env
            .iter()
            .find(|(k, _)| k == "CARGO_HOME")
            .map(|(_, v)| PathBuf::from(v))
    }

    /// Starts `spec` in the background: 202 with its info, or why not.
    pub fn start(self: &Arc<Self>, spec: RunSpec) -> (u16, Value) {
        let ok_id = !spec.run.is_empty()
            && spec.run.len() <= 64
            && spec
                .run
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok_id {
            return (
                400,
                json!({ "error": "a run ID is 1 to 64 letters, digits and `-`" }),
            );
        }
        if spec.sha.len() != 40 || !spec.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return (400, json!({ "error": "sha must be a full commit SHA" }));
        }
        if bench::scratch_owner(&spec.reference).is_none() {
            return (
                400,
                json!({ "error": "ref must be a session's scratch ref" }),
            );
        }
        if !spec
            .base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
        {
            return (400, json!({ "error": "base is not a branch name" }));
        }
        let minutes = match bench::timeout(spec.timeout) {
            Ok(m) => m,
            Err(e) => return (400, json!({ "error": e })),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let info = RunInfo {
            run: spec.run.clone(),
            state: State::Syncing,
            exit: None,
            seconds: 0,
            sha: spec.sha.clone(),
            cleaned: false,
            target_mb: None,
            free_mb: None,
        };
        {
            let mut inner = self.inner();
            if let Some(current) = &inner.current {
                return (
                    409,
                    json!({ "error": format!("wait: this bench is running {current}") }),
                );
            }
            if inner.runs.contains_key(&spec.run) {
                return (409, json!({ "error": format!("run {} exists", spec.run) }));
            }
            inner.current = Some(spec.run.clone());
            inner.runs.insert(
                spec.run.clone(),
                (info.clone(), Arc::clone(&cancel), Instant::now()),
            );
        }
        let bench = Arc::clone(self);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
            let end = bench.execute(&spec, &cancel, deadline);
            bench.finish(&spec.run, end);
        });
        (202, json!(info))
    }

    fn set_state(&self, run: &str, state: State) {
        if let Some((info, _, _)) = self.inner().runs.get_mut(run) {
            info.state = state;
        }
    }

    fn set_cleaned(&self, run: &str) {
        if let Some((info, _, _)) = self.inner().runs.get_mut(run) {
            info.cleaned = true;
        }
    }

    fn finish(&self, run: &str, (state, exit): (State, Option<i32>)) {
        let (target_mb, free_mb) = self.disk();
        let mut inner = self.inner();
        if let Some((info, _, started)) = inner.runs.get_mut(run) {
            info.state = state;
            info.exit = exit;
            info.seconds = started.elapsed().as_secs();
            info.target_mb = target_mb;
            info.free_mb = free_mb;
        }
        if inner.current.as_deref() == Some(run) {
            inner.current = None;
        }
    }

    /// The target directory's size and the free disk beside it, in MB.
    fn disk(&self) -> (Option<u64>, Option<u64>) {
        let target = self.target();
        let size = Proc::new("du")
            .arg("-sxm")
            .arg(&target)
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse().ok())
            });
        let probe = if target.exists() {
            target
        } else {
            self.work.clone()
        };
        let free = Proc::new("df")
            .arg("-Pk")
            .arg(&probe)
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .nth(1)
                    .and_then(|l| l.split_whitespace().nth(3).map(str::to_string))
                    .and_then(|kb| kb.parse::<u64>().ok())
                    .map(|kb| kb / 1024)
            });
        (size, free)
    }

    /// Syncs, guards the disk, resolves the command and runs it: the end state and exit code.
    fn execute(
        &self,
        spec: &RunSpec,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> (State, Option<i32>) {
        let log = match self.open_log(&spec.run) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("bench: log: {e}");
                return (State::Failed, None);
            }
        };
        let note = |text: &str| {
            let _ = (&log).write_all(text.as_bytes());
        };
        // A step refused at a cancel or the deadline ends the run as that, not as a failure.
        let stopped = || {
            if cancel.load(Ordering::SeqCst) {
                State::Cancelled
            } else if Instant::now() >= deadline {
                State::TimedOut
            } else {
                State::Failed
            }
        };
        if let Err(e) = self.own() {
            note(&format!("{e}\n"));
            return (stopped(), None);
        }
        if let Err(e) = self.sync(spec, &log, cancel, deadline) {
            note(&format!("{e}\n"));
            return (stopped(), None);
        }
        let (target_mb, free_mb) = self.disk();
        if bench::needs_clean(target_mb.unwrap_or(0), free_mb.unwrap_or(u64::MAX)) {
            note("bench: the disk guard cleans the workspace's crates\n");
            if let Err(e) = self.clean_workspace(&log, cancel, deadline) {
                note(&format!("{e}\n"));
                return (stopped(), None);
            }
            self.set_cleaned(&spec.run);
        }
        let steps = match self.resolve(&spec.command, &log, cancel, deadline) {
            Ok(steps) => steps,
            Err(e) => {
                note(&format!("{e}\n"));
                return (stopped(), None);
            }
        };
        self.set_state(&spec.run, State::Running);
        for step in &steps {
            note(&format!("$ {}\n", step.argv.join(" ")));
            match self.step(step, Mode::Command, &[], &log, cancel, deadline) {
                Ended::Exited(status) if status.success() => {}
                Ended::Exited(status) => return (State::Failed, status.code()),
                Ended::Cancelled => return (State::Cancelled, None),
                Ended::TimedOut => {
                    note("bench: killed at its timeout\n");
                    return (State::TimedOut, None);
                }
                Ended::NotStarted(e) => {
                    note(&format!("bench: {} did not start: {e}\n", step.argv[0]));
                    return (stopped(), None);
                }
            }
        }
        (State::Passed, Some(0))
    }

    fn open_log(&self, run: &str) -> std::io::Result<File> {
        let path = self.log_path(run);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// Hands the checkout, the target and the cargo home to uid 1000, once.
    fn own(&self) -> Result<(), String> {
        let checkout = self.checkout();
        std::fs::create_dir_all(&checkout).map_err(|e| format!("bench: {e}"))?;
        if self.sandbox == Sandbox::Plain || self.inner().owned {
            self.inner().owned = true;
            return Ok(());
        }
        let target = self.target();
        let _ = std::fs::create_dir_all(&target);
        let mut argv: Vec<String> = vec![
            "chown".into(),
            "-R".into(),
            format!("{UID}:{UID}"),
            checkout.to_string_lossy().into_owned(),
            target.to_string_lossy().into_owned(),
        ];
        if let Some(home) = self.cargo_home() {
            argv.push(home.to_string_lossy().into_owned());
        }
        let out = Proc::new(&self.sandbox.wrap(&argv, Mode::Own)[0])
            .args(&self.sandbox.wrap(&argv, Mode::Own)[1..])
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .output()
            .map_err(|e| format!("bench: chown did not start: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "bench: chown to uid {UID} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        self.inner().owned = true;
        Ok(())
    }

    fn fetch_env(spec: &RunSpec) -> Vec<(String, String)> {
        vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            (
                "GIT_CONFIG_KEY_0".into(),
                format!("http.{}.extraHeader", spec.remote),
            ),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: Bearer {}", spec.token),
            ),
        ]
    }

    /// Fetches the scratch ref and `base`, checks out `sha`, and fetches crates when the
    /// lockfile changed.
    fn sync(
        &self,
        spec: &RunSpec,
        log: &File,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        let root = String::new();
        let git = |args: &[&str], mode: Mode, extra: &[(String, String)]| -> Result<(), String> {
            let mut argv: Vec<String> = GIT_SAFE.iter().map(|a| (*a).to_string()).collect();
            argv.extend(args.iter().map(|a| (*a).to_string()));
            let step = Step {
                dir: root.clone(),
                argv,
            };
            match self.step(&step, mode, extra, log, cancel, deadline) {
                Ended::Exited(s) if s.success() => Ok(()),
                Ended::Exited(_) => Err(format!("bench: `git {}` failed", args[0])),
                Ended::Cancelled => Err("bench: cancelled".into()),
                Ended::TimedOut => Err("bench: timed out while syncing".into()),
                Ended::NotStarted(e) => Err(format!("bench: git did not start: {e}")),
            }
        };
        if self.checkout().join(".git").exists() {
            clear_locks(&self.checkout().join(".git"));
        } else {
            git(&["init", "-q"], Mode::Command, &[])?;
        }
        let head = format!("+{}:refs/bench/head", spec.reference);
        let base = format!("+refs/heads/{0}:refs/remotes/artifacts/{0}", spec.base);
        git(
            &["fetch", "-q", "--no-tags", &spec.remote, &head, &base],
            Mode::Fetch,
            &Self::fetch_env(spec),
        )?;
        let mut rev: Vec<String> = GIT_SAFE.iter().map(|a| (*a).to_string()).collect();
        rev.extend(["rev-parse", "refs/bench/head"].map(String::from));
        let fetched = self
            .capture(
                &Step {
                    dir: String::new(),
                    argv: rev,
                },
                cancel,
                deadline,
            )
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        if fetched != spec.sha {
            return Err(format!(
                "bench: {}'s head is {fetched}, not {}; push again",
                spec.reference, spec.sha
            ));
        }
        git(
            &[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "-q",
                "--force",
                &spec.sha,
            ],
            Mode::Command,
            &[],
        )?;
        git(&["clean", "-qffdx"], Mode::Command, &[])?;
        self.inner().sha = Some(spec.sha.clone());
        let ws = self.workspace();
        let lock = std::fs::read(self.checkout().join(&ws).join("Cargo.lock")).ok();
        let hash = lock.map(|bytes| {
            let mut h = DefaultHasher::new();
            bytes.hash(&mut h);
            h.finish()
        });
        if hash.is_some() && hash != self.inner().lock {
            let step = Step {
                dir: ws,
                argv: ["cargo", "fetch", "--locked"].map(String::from).to_vec(),
            };
            match self.step(&step, Mode::Fetch, &[], log, cancel, deadline) {
                Ended::Exited(s) if s.success() => self.inner().lock = hash,
                _ => return Err("bench: `cargo fetch --locked` failed".into()),
            }
        }
        Ok(())
    }

    /// `cargo clean -p` for each of the workspace's own crates; the warmed dependencies stay.
    fn clean_workspace(
        &self,
        log: &File,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), String> {
        let ws = self.workspace();
        if !self.checkout().join(&ws).join("Cargo.toml").exists() {
            return Ok(());
        }
        let metadata = Step {
            dir: ws.clone(),
            argv: [
                "cargo",
                "metadata",
                "--no-deps",
                "--offline",
                "--format-version",
                "1",
            ]
            .map(String::from)
            .to_vec(),
        };
        let text = self.capture(&metadata, cancel, deadline)?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("bench: cargo metadata doesn't parse: {e}"))?;
        let names: Vec<String> = v["packages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect();
        if names.is_empty() {
            return Ok(());
        }
        let mut argv = vec!["cargo".to_string(), "clean".to_string()];
        for n in names {
            argv.push("-p".into());
            argv.push(n);
        }
        let step = Step { dir: ws, argv };
        match self.step(&step, Mode::Command, &[], log, cancel, deadline) {
            Ended::Exited(s) if s.success() => Ok(()),
            _ => Err("bench: `cargo clean` failed".into()),
        }
    }

    /// The steps `command` runs.
    fn resolve(
        &self,
        command: &Command,
        log: &File,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Vec<Step>, String> {
        match command {
            Command::Argv { argv } => {
                bench::allow::allow(argv).map_err(|r| r.to_string())?;
                Ok(vec![Step {
                    dir: argv_dir(argv, &self.workspace()),
                    argv: argv.clone(),
                }])
            }
            Command::Task { name } => {
                let text = match std::fs::read_to_string(self.checkout().join("mise.toml")) {
                    Ok(t) => Some(t),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(format!("bench: mise.toml: {e}")),
                };
                task_steps(text.as_deref(), name)
            }
            Command::Check { name } => {
                let _ = (&*log).write_all(b"bench: reading this change's checks from scope\n");
                let ws = self.workspace();
                let root = root_from(&ws);
                let scope = Step {
                    dir: ws.clone(),
                    argv: scope_argv(&root),
                };
                let text = self.capture(&scope, cancel, deadline)?;
                check_step(&text, name, &ws).map(|s| vec![s])
            }
        }
    }

    fn command_env(&self, extra: &[(String, String)], mode: Mode) -> Vec<(String, String)> {
        let mut env = self.env.clone();
        env.push(("HOME".into(), "/tmp".into()));
        for (k, v) in bench::allow::ENV {
            if mode == Mode::Fetch && k == "CARGO_NET_OFFLINE" {
                continue;
            }
            env.push((k.to_string(), v.to_string()));
        }
        env.extend(extra.iter().cloned());
        env
    }

    fn spawn(
        &self,
        step: &Step,
        mode: Mode,
        extra: &[(String, String)],
        out: Stdio,
        err: Stdio,
    ) -> std::io::Result<std::process::Child> {
        let argv = self.sandbox.wrap(&step.argv, mode);
        let dir = if step.dir.is_empty() {
            self.checkout()
        } else {
            self.checkout().join(&step.dir)
        };
        Proc::new(&argv[0])
            .args(&argv[1..])
            .current_dir(dir)
            .env_clear()
            .envs(self.command_env(extra, mode))
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
    }

    /// Runs `step` with its output appended to `log`.
    fn step(
        &self,
        step: &Step,
        mode: Mode,
        extra: &[(String, String)],
        log: &File,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Ended {
        let (out, err) = match (log.try_clone(), log.try_clone()) {
            (Ok(o), Ok(e)) => (o, e),
            (Err(e), _) | (_, Err(e)) => return Ended::NotStarted(e.to_string()),
        };
        match self.spawn(step, mode, extra, Stdio::from(out), Stdio::from(err)) {
            Ok(child) => wait(child, cancel, deadline),
            Err(e) => Ended::NotStarted(e.to_string()),
        }
    }

    /// Runs `step` with no network and returns its stdout; its stderr goes nowhere.
    fn capture(
        &self,
        step: &Step,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<String, String> {
        let dir = self.work.join("runs");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("capture-{}.out", std::process::id()));
        let file = File::create(&path).map_err(|e| format!("bench: {e}"))?;
        let ended = match self.spawn(step, Mode::Command, &[], Stdio::from(file), Stdio::null()) {
            Ok(child) => wait(child, cancel, deadline),
            Err(e) => Ended::NotStarted(e.to_string()),
        };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        match ended {
            Ended::Exited(s) if s.success() => Ok(text),
            _ => Err(format!("bench: `{}` failed", step.argv.join(" "))),
        }
    }

    /// A run's info and its log from `offset`, waiting up to `wait` seconds for new output.
    pub fn status(&self, body: &[u8]) -> (u16, Value) {
        #[derive(Deserialize)]
        struct Ask {
            run: String,
            #[serde(default)]
            offset: u64,
            #[serde(default)]
            wait: u32,
        }
        let ask: Ask = match serde_json::from_slice(body) {
            Ok(a) => a,
            Err(e) => {
                return (
                    400,
                    json!({ "error": format!("not a status request: {e}") }),
                );
            }
        };
        let wait = Duration::from_secs(u64::from(ask.wait.min(bench::MAX_WAIT_SECONDS)));
        let until = Instant::now() + wait;
        loop {
            let Some(info) = self.info(&ask.run) else {
                return (404, json!({ "error": format!("no run {}", ask.run) }));
            };
            let page = self.page(&ask.run, ask.offset);
            if !page.is_empty() || info.state.done() || Instant::now() >= until {
                let mut v = json!(info);
                v["log"] = json!(String::from_utf8_lossy(&page));
                v["next"] = json!(ask.offset + page.len() as u64);
                return (200, v);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn info(&self, run: &str) -> Option<RunInfo> {
        self.inner().runs.get(run).map(|(info, _, started)| {
            let mut info = info.clone();
            if !info.state.done() {
                info.seconds = started.elapsed().as_secs();
            }
            info
        })
    }

    fn page(&self, run: &str, offset: u64) -> Vec<u8> {
        let Ok(mut f) = File::open(self.log_path(run)) else {
            return Vec::new();
        };
        if f.seek(SeekFrom::Start(offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        let _ = f.take(bench::LOG_PAGE as u64).read_to_end(&mut buf);
        let keep = whole_utf8(&buf).len();
        buf.truncate(keep);
        buf
    }

    /// Cancels a run.
    pub fn cancel(&self, body: &[u8]) -> (u16, Value) {
        let run = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| v["run"].as_str().map(str::to_string))
            .unwrap_or_default();
        match self.inner().runs.get(&run) {
            Some((_, cancel, _)) => {
                cancel.store(true, Ordering::SeqCst);
                (202, json!({ "run": run, "cancelling": true }))
            }
            None => (404, json!({ "error": format!("no run {run}") })),
        }
    }

    /// The soft reset: an untracked-file clean of the checkout and the workspace crates' clean.
    pub fn reset(&self) -> (u16, Value) {
        if let Some(current) = self.inner().current.clone() {
            return (
                409,
                json!({ "error": format!("wait: this bench is running {current}") }),
            );
        }
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(600);
        let log = match self.open_log("reset") {
            Ok(f) => f,
            Err(e) => return (500, json!({ "error": e.to_string() })),
        };
        if self.checkout().join(".git").exists() {
            let step = Step {
                dir: String::new(),
                argv: GIT_SAFE
                    .iter()
                    .chain(["clean", "-qffdx"].iter())
                    .map(|a| (*a).to_string())
                    .collect(),
            };
            let _ = self.step(&step, Mode::Command, &[], &log, &cancel, deadline);
            if let Err(e) = self.clean_workspace(&log, &cancel, deadline) {
                return (500, json!({ "error": e }));
            }
        }
        self.inner().lock = None;
        let (target_mb, free_mb) = self.disk();
        (
            200,
            json!({ "reset": true, "target_mb": target_mb, "free_mb": free_mb }),
        )
    }

    /// The bench's synced commit, its running run, and its disk.
    #[must_use]
    pub fn about(&self) -> (u16, Value) {
        let (target_mb, free_mb) = self.disk();
        let inner = self.inner();
        (
            200,
            json!({
                "sha": inner.sha,
                "running": inner.current,
                "runs": inner.runs.len(),
                "target_mb": target_mb,
                "free_mb": free_mb,
            }),
        )
    }
}

/// How a step ended.
enum Ended {
    Exited(ExitStatus),
    Cancelled,
    TimedOut,
    NotStarted(String),
}

fn wait(mut child: std::process::Child, cancel: &AtomicBool, deadline: Instant) -> Ended {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ended::Exited(status),
            Ok(None) => {}
            Err(e) => return Ended::NotStarted(e.to_string()),
        }
        let why = if cancel.load(Ordering::SeqCst) {
            Some(Ended::Cancelled)
        } else if Instant::now() >= deadline {
            Some(Ended::TimedOut)
        } else {
            None
        };
        if let Some(why) = why {
            let _ = child.kill();
            let _ = child.wait();
            return why;
        }
        std::thread::sleep(TICK);
    }
}

/// `/v0/bench/<route>`'s answer.
pub fn route(bench: &Arc<Bench>, route: &str, body: &[u8]) -> (u16, Value) {
    match route {
        "run" => match serde_json::from_slice::<RunSpec>(body) {
            Ok(spec) => bench.start(spec),
            Err(e) => (400, json!({ "error": format!("not a run: {e}") })),
        },
        "status" => bench.status(body),
        "cancel" => bench.cancel(body),
        "reset" => bench.reset(),
        "info" => bench.about(),
        _ => (404, json!({ "error": "no such bench route" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deploy::scope::WORKSPACE;
    use std::path::Path;

    const MISE: &str = r#"
[tasks.fmt]
run = "echo formatted"

[tasks.lint]
depends = ["fmt"]
run = ["echo linted", "echo twice"]
dir = "src/pigeon-cli"

[tasks.test]
depends = ["fmt", "lint"]
run = "echo testing; exit 3"

[tasks.build]
depends = ["deploy"]
run = "true"

[tasks.rust]
depends = ["rust"]

[tasks.features]
dir = "../outside"
run = "true"

[tasks.policy]
run = "sleep 30"
"#;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn the_sandbox_cuts_the_network_except_to_fetch() {
        let argv = s(&["cargo", "test"]);
        let command = Sandbox::Unshare.wrap(&argv, Mode::Command);
        assert!(command.contains(&"--net".to_string()));
        assert!(command.contains(&"--map-auto".to_string()));
        assert!(command.iter().any(|a| a.contains("setpriv --reuid=1000")));
        assert!(command.iter().any(|a| a.contains("mount -t tmpfs")));
        assert!(command.iter().any(|a| a.contains("ip link set lo up")));
        assert_eq!(&command[command.len() - 2..], &argv[..]);
        let fetch = Sandbox::Unshare.wrap(&argv, Mode::Fetch);
        assert!(!fetch.contains(&"--net".to_string()));
        assert!(!fetch.iter().any(|a| a.contains("ip link")));
        assert!(fetch.iter().any(|a| a.contains("setpriv --reuid=1000")));
        let own = Sandbox::Unshare.wrap(&argv, Mode::Own);
        assert!(own.contains(&"--net".to_string()));
        assert!(!own.iter().any(|a| a.contains("setpriv")));
        assert_eq!(Sandbox::Plain.wrap(&argv, Mode::Command), argv);
    }

    #[test]
    fn the_judge_build_is_kept_out_of_tmp_and_handed_to_commands() {
        let dir = tempfile::tempdir().expect("dir");
        let from = dir.path().join("downloaded");
        std::fs::write(&from, "#!/bin/sh\n").expect("write");
        let from = from.to_string_lossy().into_owned();
        let work = dir.path().join("work");
        let b = Bench::new(work.clone(), Sandbox::Plain, |k| {
            (k == crate::eviction::VAR).then(|| from.clone())
        });
        let kept = work.join("bin").join(crate::eviction::BINARY);
        assert!(kept.is_file());
        let want = (
            crate::eviction::VAR.to_string(),
            kept.to_string_lossy().into_owned(),
        );
        assert!(b.command_env(&[], Mode::Command).contains(&want));
        let missing = Bench::new(dir.path().join("w2"), Sandbox::Plain, |k| {
            (k == crate::eviction::VAR).then(|| "/no/such/judge".to_string())
        });
        let env = missing.command_env(&[], Mode::Command);
        assert!(!env.iter().any(|(k, _)| k == crate::eviction::VAR));
    }

    #[test]
    fn mise_tasks_run_their_depends_first_and_once() {
        let steps = mise_steps(MISE, "test").expect("resolves");
        let runs: Vec<(&str, &str)> = steps
            .iter()
            .map(|st| (st.dir.as_str(), st.argv[2].as_str()))
            .collect();
        assert_eq!(
            runs,
            vec![
                ("", "echo formatted"),
                ("src/pigeon-cli", "echo linted"),
                ("src/pigeon-cli", "echo twice"),
                ("", "echo testing; exit 3"),
            ]
        );
        assert!(steps.iter().all(|st| st.argv[..2] == s(&["sh", "-c"])[..]));
    }

    #[test]
    fn mise_tasks_outside_the_list_loops_and_escapes_are_refused() {
        assert_eq!(
            mise_steps(MISE, "build").unwrap_err(),
            "bench: mise task `build` depends on `deploy`, which isn't allowed"
        );
        assert_eq!(
            mise_steps(MISE, "ci").unwrap_err(),
            "bench: mise task `ci` isn't allowed"
        );
        assert_eq!(
            mise_steps(MISE, "rust").unwrap_err(),
            "bench: mise task `rust` depends on itself"
        );
        assert_eq!(
            mise_steps(MISE, "features").unwrap_err(),
            "bench: mise task `features`'s dir leaves the checkout"
        );
        assert_eq!(
            mise_steps(MISE, "instance").unwrap_err(),
            "bench: mise.toml has no task `instance`"
        );
        assert!(mise_steps("not toml [", "fmt").is_err());
    }

    #[test]
    fn without_mise_toml_a_task_runs_the_commits_own_pigeon_dev_from_its_root() {
        let steps = task_steps(None, "policy").expect("allowed");
        assert_eq!(
            steps,
            vec![Step {
                dir: String::new(),
                argv: s(&[
                    "cargo", "run", "-q", "--locked", "--bin", "pigeon", "--", "dev", "policy"
                ]),
            }]
        );
        assert_eq!(
            task_steps(None, "deploy").unwrap_err(),
            "bench: task `deploy` isn't allowed"
        );
        assert_eq!(
            task_steps(Some(MISE), "fmt").expect("mise"),
            mise_steps(MISE, "fmt").expect("mise")
        );
    }

    #[test]
    fn scope_finds_the_root_from_either_workspace_layout() {
        assert_eq!(root_from(""), ".");
        assert_eq!(root_from(WORKSPACE), "../..");
    }

    #[test]
    fn cargo_runs_in_the_workspace_and_pigeon_and_git_at_the_root() {
        assert_eq!(argv_dir(&s(&["cargo", "test"]), WORKSPACE), WORKSPACE);
        assert_eq!(argv_dir(&s(&["pigeon", "dev", "scope"]), WORKSPACE), "");
        assert_eq!(argv_dir(&s(&["git", "status"]), WORKSPACE), "");
    }

    #[test]
    fn a_check_is_found_by_name_in_scope() {
        let json = r#"[{"name":"fmt","argv":["cargo","fmt","--check"]},{"name":"test pigeon: suite","argv":["sh","-c","cargo test"]}]"#;
        let step = check_step(json, "test pigeon: suite", WORKSPACE).expect("found");
        assert_eq!(step.dir, WORKSPACE);
        assert_eq!(step.argv, s(&["sh", "-c", "cargo test"]));
        assert_eq!(
            check_step(json, "wasm", WORKSPACE).unwrap_err(),
            "bench: no check `wasm` in this change's scope"
        );
    }

    #[test]
    fn a_page_ends_on_whole_utf8() {
        let text = "aé".as_bytes();
        assert_eq!(whole_utf8(text), text);
        assert_eq!(whole_utf8(&text[..2]), b"a");
        assert_eq!(whole_utf8(b""), b"");
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Proc::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    const REF: &str = "refs/heads/scratch/session_test";

    /// A bare remote with `main` and a scratch ref holding mise.toml and the workspace dir.
    fn remote(root: &Path) -> (PathBuf, String) {
        let bare = root.join("remote.git");
        let src = root.join("src");
        std::fs::create_dir_all(src.join(WORKSPACE)).expect("dirs");
        git(
            root,
            &["init", "-q", "--bare", bare.to_str().expect("utf8")],
        );
        git(root, &["init", "-q", src.to_str().expect("utf8")]);
        std::fs::write(src.join("mise.toml"), MISE).expect("mise");
        std::fs::write(src.join(WORKSPACE).join("keep"), "").expect("keep");
        git(&src, &["add", "-A"]);
        git(&src, &["commit", "-qm", "one"]);
        let sha = git(&src, &["rev-parse", "HEAD"]);
        let url = bare.to_str().expect("utf8").to_string();
        git(
            &src,
            &[
                "push",
                "-q",
                &url,
                "HEAD:refs/heads/main",
                &format!("HEAD:{REF}"),
            ],
        );
        (bare, sha)
    }

    fn spec(remote: &Path, sha: &str, run: &str, argv: &[&str]) -> RunSpec {
        RunSpec {
            run: run.into(),
            remote: remote.to_str().expect("utf8").into(),
            token: "a-token-that-is-never-logged".into(),
            reference: REF.into(),
            sha: sha.into(),
            base: "main".into(),
            command: bench::allow::allow(&s(argv)).expect("allowed"),
            timeout: Some(1),
        }
    }

    fn bench(work: &Path) -> Arc<Bench> {
        let path = std::env::var("PATH").ok();
        Arc::new(Bench::new(work.to_path_buf(), Sandbox::Plain, move |k| {
            (k == "PATH").then(|| path.clone()).flatten()
        }))
    }

    fn done(b: &Bench, run: &str) -> Value {
        let mut offset = 0;
        let mut log = String::new();
        for _ in 0..200 {
            let (code, v) = b.status(
                json!({ "run": run, "offset": offset, "wait": 1 })
                    .to_string()
                    .as_bytes(),
            );
            assert_eq!(code, 200, "{v}");
            log.push_str(v["log"].as_str().unwrap_or(""));
            offset = v["next"].as_u64().expect("next");
            if v["state"] != "syncing" && v["state"] != "running" {
                let mut v = v;
                v["log"] = json!(log);
                return v;
            }
        }
        panic!("run {run} never ended");
    }

    #[test]
    fn a_run_syncs_the_scratch_ref_runs_its_task_and_pages_its_log() {
        let root = tempfile::tempdir().expect("tmp");
        let (bare, sha) = remote(root.path());
        let b = bench(&root.path().join("work"));
        let (code, v) = b.start(spec(&bare, &sha, "r1", &["pigeon", "dev", "lint"]));
        assert_eq!(code, 202, "{v}");
        let v = done(&b, "r1");
        assert_eq!(v["state"], "passed", "{v}");
        assert_eq!(v["exit"], 0);
        let log = v["log"].as_str().expect("log");
        assert!(
            log.contains("formatted\n") && log.contains("linted\n") && log.contains("twice\n"),
            "{log}"
        );
        assert!(!log.contains("a-token-that-is-never-logged"));
        assert_eq!(b.about().1["sha"], json!(sha));
        let (_, page) = b.status(json!({ "run": "r1", "offset": 3 }).to_string().as_bytes());
        assert!(log[3..].starts_with(page["log"].as_str().expect("page")));
    }

    #[test]
    fn locks_a_killed_git_left_do_not_stop_the_next_run() {
        let root = tempfile::tempdir().expect("tmp");
        let (bare, sha) = remote(root.path());
        let work = root.path().join("work");
        let b = bench(&work);
        b.start(spec(&bare, &sha, "r1", &["git", "status"]));
        assert_eq!(done(&b, "r1")["state"], "passed");
        let git_dir = work.join("pigeon").join(".git");
        std::fs::write(git_dir.join("index.lock"), "").expect("lock");
        std::fs::create_dir_all(git_dir.join("refs/bench")).expect("refs");
        std::fs::write(git_dir.join("refs/bench/head.lock"), "").expect("ref lock");
        b.start(spec(&bare, &sha, "r2", &["git", "status"]));
        let v = done(&b, "r2");
        assert_eq!(v["state"], "passed", "{v}");
        assert!(!git_dir.join("index.lock").exists());
    }

    #[test]
    fn a_failing_task_reports_its_exit_code() {
        let root = tempfile::tempdir().expect("tmp");
        let (bare, sha) = remote(root.path());
        let b = bench(&root.path().join("work"));
        b.start(spec(&bare, &sha, "r2", &["pigeon", "dev", "test"]));
        let v = done(&b, "r2");
        assert_eq!(v["state"], "failed", "{v}");
        assert_eq!(v["exit"], 3);
    }

    #[test]
    fn a_sha_that_is_not_the_refs_head_fails_the_sync() {
        let root = tempfile::tempdir().expect("tmp");
        let (bare, _) = remote(root.path());
        let b = bench(&root.path().join("work"));
        let other = "0123456789012345678901234567890123456789";
        b.start(spec(&bare, other, "r3", &["git", "status"]));
        let v = done(&b, "r3");
        assert_eq!(v["state"], "failed");
        assert!(
            v["log"].as_str().expect("log").contains("push again"),
            "{v}"
        );
    }

    #[test]
    fn one_run_at_a_time_and_a_cancel_stops_it() {
        let root = tempfile::tempdir().expect("tmp");
        let (bare, sha) = remote(root.path());
        let b = bench(&root.path().join("work"));
        assert_eq!(
            b.start(spec(&bare, &sha, "long", &["pigeon", "dev", "policy"]))
                .0,
            202
        );
        let (code, v) = b.start(spec(&bare, &sha, "next", &["git", "status"]));
        assert_eq!(code, 409);
        assert!(v["error"].as_str().expect("why").starts_with("wait: "));
        assert_eq!(b.reset().0, 409);
        assert_eq!(b.cancel(br#"{"run":"long"}"#).0, 202);
        assert_eq!(done(&b, "long")["state"], "cancelled");
        assert_eq!(
            b.start(spec(&bare, &sha, "next", &["git", "status"])).0,
            202
        );
        assert_eq!(done(&b, "next")["state"], "passed");
        assert_eq!(b.reset().0, 200);
    }

    #[test]
    fn bad_runs_are_refused_before_starting() {
        let root = tempfile::tempdir().expect("tmp");
        let b = bench(root.path());
        let good = spec(
            Path::new("/nowhere"),
            &"a".repeat(40),
            "ok",
            &["git", "status"],
        );
        let mut bad_ref = good.clone();
        bad_ref.reference = "refs/heads/change/1-x".into();
        assert_eq!(b.start(bad_ref).0, 400);
        let mut bad_id = good.clone();
        bad_id.run = "a b".into();
        assert_eq!(b.start(bad_id).0, 400);
        let mut bad_sha = good.clone();
        bad_sha.sha = "abc".into();
        assert_eq!(b.start(bad_sha).0, 400);
        let mut bad_time = good;
        bad_time.timeout = Some(91);
        assert_eq!(b.start(bad_time).0, 400);
        assert_eq!(b.status(br#"{"run":"none"}"#).0, 404);
        assert_eq!(b.cancel(br#"{"run":"none"}"#).0, 404);
        assert_eq!(route(&b, "nope", b"{}").0, 404);
    }

    #[test]
    fn a_check_reads_its_scope_with_the_harness_dev_scope() {
        assert_eq!(
            scope_argv("x").join(" "),
            "cargo run -q --locked -p pigeon-harness -- dev scope --root x --base artifacts/main --json"
        );
    }
}
