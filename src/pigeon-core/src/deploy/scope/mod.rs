//! The checks a change needs, from the paths it touches, and what a deploy of `main` builds; the
//! selection only, from inputs the shell reads; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::release::Config;

mod map;

use super::gate::{Gate, MAX_PACKAGES};

pub use map::{Binary, TestMap, Unit};

/// The workspace's layout, from the root manifest's `[workspace.metadata.pigeon]`, which is
/// required; see the 2026-11-17 amendment to
/// `.claude/decisions/2026-11-15-development-code-checks-by-reach.md`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Layout {
    /// The workspace's directory, from the root; empty for the root. Set by the shell.
    #[serde(skip)]
    pub workspace: String,
    /// The package of the `pigeon` binary.
    pub cli: String,
    /// The CLI's files, from its crate's directory, that every feature builds.
    #[serde(default = "cli_shared")]
    pub cli_shared: Vec<String>,
    /// The package whose `worker` build compiles to wasm.
    #[serde(default)]
    pub worker: Option<Worker>,
    /// Further packages whose `worker` build compiles to wasm, each checked the same way.
    #[serde(default)]
    pub workers: Vec<Worker>,
    /// The packages whose tests read this repository's own policies.
    #[serde(default)]
    pub policy_tests: Vec<String>,
    /// The suite modules a change to the infrastructure runs.
    #[serde(default)]
    pub infra_tests: Option<InfraTests>,
    /// `pigeon`'s arguments, before an operation's name, that run it.
    #[serde(default = "run")]
    pub run: Vec<String>,
    /// `pigeon`'s arguments that validate the instance.
    #[serde(default = "instance")]
    pub instance: Vec<String>,
}

/// A package whose `worker` feature builds for wasm.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Worker {
    /// The package.
    pub package: String,
    /// Its check's name.
    pub name: String,
    /// The directories, from the root, only its wasm build compiles.
    pub only: Vec<String>,
}

/// The suite modules of `package`'s `test` target that an infrastructure change runs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct InfraTests {
    /// The package.
    pub package: String,
    /// Its test target.
    pub test: String,
    /// The module filters, each ending `::`.
    pub modules: Vec<String>,
}

fn strings(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| (*s).to_string()).collect()
}

fn cli_shared() -> Vec<String> {
    strings(&[
        "src/main.rs",
        "src/lib.rs",
        "src/cli/mod.rs",
        "src/commands/mod.rs",
        "Cargo.toml",
    ])
}

fn run() -> Vec<String> {
    strings(&["run"])
}

fn instance() -> Vec<String> {
    strings(&["instance"])
}

impl Layout {
    /// A layout at the root whose CLI is `cli`, with no worker, policy tests or infra tests.
    #[must_use]
    pub fn new(cli: &str) -> Self {
        Self {
            workspace: String::new(),
            cli: cli.into(),
            cli_shared: cli_shared(),
            worker: None,
            workers: Vec::new(),
            policy_tests: Vec::new(),
            infra_tests: None,
            run: run(),
            instance: instance(),
        }
    }

    /// `path`, from the root, as seen from the workspace.
    #[must_use]
    pub fn from_workspace(&self, path: &str) -> String {
        let depth = self.workspace.split('/').filter(|s| !s.is_empty()).count();
        format!("{}{path}", "../".repeat(depth))
    }

    /// The root, as seen from the workspace.
    #[must_use]
    pub fn root(&self) -> String {
        let up = self.from_workspace("");
        if up.is_empty() {
            ".".into()
        } else {
            up.trim_end_matches('/').into()
        }
    }

    /// `path`, from the workspace, as seen from the root.
    #[must_use]
    pub fn in_workspace(&self, path: &str) -> String {
        if self.workspace.is_empty() {
            path.into()
        } else {
            format!("{}/{path}", self.workspace.trim_end_matches('/'))
        }
    }

    /// Paths whose change runs every check.
    #[must_use]
    pub fn everything(&self) -> Vec<String> {
        let mut out: Vec<String> = ["Cargo.toml", "Cargo.lock", ".cargo/"]
            .iter()
            .map(|p| self.in_workspace(p))
            .collect();
        out.extend(["mise.toml".to_string(), RELEASES.to_string()]);
        out
    }

    /// `pigeon`'s command line for `args`, run in the workspace.
    fn pigeon(&self, args: &[&str]) -> Vec<String> {
        [
            "cargo",
            "run",
            "-q",
            "--locked",
            "-p",
            self.cli.as_str(),
            "--",
        ]
        .iter()
        .chain(args)
        .map(|s| (*s).to_string())
        .collect()
    }

    /// `pigeon`'s command line that runs operation `op` on the root.
    fn operation(&self, op: &str) -> Vec<String> {
        let root = self.root();
        let mut args: Vec<&str> = self.run.iter().map(String::as_str).collect();
        args.extend([op, "--root", root.as_str()]);
        self.pigeon(&args)
    }

    /// Whether only the wasm `worker` build compiles `path`.
    fn wasm_only(&self, path: &str) -> bool {
        self.worker
            .iter()
            .chain(self.workers.iter())
            .any(|w| w.only.iter().any(|d| path.starts_with(d.as_str())))
    }
}

/// The release configuration, from the root.
pub const RELEASES: &str = ".config/releases.yaml";

/// Paths whose change validates the instance's infrastructure.
const INFRA: [&str; 6] = [
    "catalog/",
    "infra/",
    "workloads/",
    "config.yaml",
    ".config/providers",
    ".config/instance.yaml",
];

/// Paths whose change can break the tests that read this repository's own policies.
const POLICIES: [&str; 1] = [".config/policies/"];

/// Paths whose change rebuilds the sites.
const SITES: [&str; 5] = [
    "src/site-templates/",
    "src/pigeon-site-templates/",
    "workloads/",
    "sites/",
    "config.yaml",
];

/// The iOS app, which its `ios` check builds on macOS; see
/// `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`.
const IOS: [&str; 1] = ["src/pigeon-ios/"];

/// One check: a name, and the command to run in the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    /// What it checks.
    pub name: String,
    /// The command, run in the workspace.
    pub argv: Vec<String>,
    /// For a `cargo test` check: the tests it runs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<Unit>,
}

fn job(name: impl Into<String>, argv: &[&str]) -> Job {
    Job {
        name: name.into(),
        argv: argv.iter().map(|s| (*s).to_string()).collect(),
        units: Vec::new(),
    }
}

/// The workspace's crates, as the shell reads them from its manifests: each crate directory's package, and each package's workspace
/// dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crates {
    /// Each crate directory's package.
    pub by_dir: BTreeMap<String, String>,
    /// Each package's workspace dependencies.
    pub deps: BTreeMap<String, BTreeSet<String>>,
    /// `pigeon`'s features, each with the packages it enables.
    pub features: BTreeMap<String, BTreeSet<String>>,
    /// Each package's integration test targets, where known; a map unit naming one that is gone
    /// runs as its `suite` module instead.
    pub tests: BTreeMap<String, BTreeSet<String>>,
    /// Each package's binary targets, where known; a map unit naming one that is gone picks none.
    pub bins: BTreeMap<String, BTreeSet<String>>,
    /// Where the workspace and its special crates are.
    pub layout: Layout,
}

impl Crates {
    /// Those of `packages` the workspace has.
    #[must_use]
    pub fn of<'a>(&self, packages: &[&'a str]) -> Vec<&'a str> {
        packages
            .iter()
            .copied()
            .filter(|p| self.by_dir.values().any(|n| n == p))
            .collect()
    }

    /// The CLI crate's directory, from the root, when the workspace has it.
    fn cli_dir(&self) -> Option<&str> {
        self.by_dir
            .iter()
            .find(|(_, p)| **p == self.layout.cli)
            .map(|(d, _)| d.as_str())
    }

    /// The wasm worker packages the workspace has.
    fn workers(&self) -> impl Iterator<Item = &Worker> {
        self.layout
            .worker
            .iter()
            .chain(self.layout.workers.iter())
            .filter(|w| self.by_dir.values().any(|p| *p == w.package))
    }
}

impl Crates {
    /// The `pigeon` features whose build `files` and the `affected` packages change: a feature
    /// whose package is affected, whose `commands/<f>.rs` or `cli/<f>.rs` changed, or every one
    /// when a file all commands share changed.
    fn pigeon_features_touched(
        &self,
        affected: &BTreeSet<String>,
        files: &[&str],
    ) -> BTreeSet<String> {
        let pigeon: Vec<&str> = match self.cli_dir() {
            Some(prefix) => files
                .iter()
                .filter_map(|f| f.strip_prefix(prefix))
                .collect(),
            None => Vec::new(),
        };
        if pigeon
            .iter()
            .any(|f| self.layout.cli_shared.iter().any(|s| s == f))
        {
            return self.features.keys().cloned().collect();
        }
        self.features
            .iter()
            .filter(|(f, packages)| {
                packages.iter().any(|p| affected.contains(p))
                    || pigeon.iter().any(|p| {
                        *p == format!("src/commands/{f}.rs") || *p == format!("src/cli/{f}.rs")
                    })
            })
            .map(|(f, _)| f.clone())
            .collect()
    }

    /// `changed` and every crate that depends on one of them, transitively.
    fn with_dependants(&self, changed: &BTreeSet<String>) -> BTreeSet<String> {
        let mut out = changed.clone();
        loop {
            let more: Vec<String> = self
                .deps
                .iter()
                .filter(|(c, d)| !out.contains(*c) && d.iter().any(|x| out.contains(x)))
                .map(|(c, _)| c.clone())
                .collect();
            if more.is_empty() {
                return out;
            }
            out.extend(more);
        }
    }
}

fn under(path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| path.starts_with(p))
}

/// The full set, for a change to a workspace-wide setting.
fn everything(features: &BTreeSet<String>, crates: &Crates) -> Vec<Job> {
    let mut out = vec![
        job("fmt", &["cargo", "fmt", "--all", "--", "--check"]),
        job(
            "clippy",
            &[
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
    ];
    let cli = crates.layout.cli.as_str();
    out.push(job(
        format!("clippy {cli} (no features)"),
        &[
            "cargo",
            "clippy",
            "-p",
            cli,
            "--all-targets",
            "--locked",
            "--no-default-features",
            "--",
            "-D",
            "warnings",
        ],
    ));
    out.extend(features.iter().map(|f| feature_clippy(cli, f)));
    out.extend(crates.workers().map(worker_job));
    out.push(workspace_tests(crates));
    let root = crates.layout.root();
    let mut instance: Vec<&str> = crates.layout.instance.iter().map(String::as_str).collect();
    instance.extend(["--root", root.as_str()]);
    out.push(Job {
        name: "pigeon instance".into(),
        argv: crates.layout.pigeon(&instance),
        units: Vec::new(),
    });
    out
}

fn feature_clippy(cli: &str, f: &String) -> Job {
    job(
        format!("clippy {cli} --features {f}"),
        &[
            "cargo",
            "clippy",
            "-p",
            cli,
            "--all-targets",
            "--locked",
            "--no-default-features",
            "--features",
            f,
            "--",
            "-D",
            "warnings",
        ],
    )
}

/// The wasm `worker` build's check for `worker`.
fn worker_job(worker: &Worker) -> Job {
    job(
        worker.name.as_str(),
        &[
            "cargo",
            "clippy",
            "-p",
            worker.package.as_str(),
            "--lib",
            "--locked",
            "--no-default-features",
            "--features",
            "worker",
            "--target",
            "wasm32-unknown-unknown",
            "--",
            "-D",
            "warnings",
        ],
    )
}

/// The Rust checks for the crates `files` touch and their dependants; with `map`, their tests are
/// the binaries [`TestMap::select`] picks. A wasm-only path adds only its crate's wasm clippy.
fn rust_jobs(crates: &Crates, config: &Config, files: &[&str], map: Option<&TestMap>) -> Vec<Job> {
    let mut out = Vec::new();
    let layout = &crates.layout;
    let all = files;
    let wasm = files.iter().any(|f| layout.wasm_only(f));
    let native: Vec<&str> = files
        .iter()
        .copied()
        .filter(|f| !layout.wasm_only(f))
        .collect();
    let touched = |w: &Worker| {
        all.iter()
            .any(|f| w.only.iter().any(|d| f.starts_with(d.as_str())))
    };
    let files = native.as_slice();
    let changed: BTreeSet<String> = files
        .iter()
        .filter_map(|f| {
            crates
                .by_dir
                .iter()
                .find(|(dir, _)| f.starts_with(dir.as_str()))
                .map(|(_, c)| c.clone())
        })
        .collect();
    let affected = crates.with_dependants(&changed);
    if affected.is_empty() && wasm {
        out.push(job("fmt", &["cargo", "fmt", "--all", "--", "--check"]));
        out.extend(crates.workers().filter(|w| touched(w)).map(worker_job));
    }
    if !affected.is_empty() {
        let packages: Vec<&str> = affected.iter().map(String::as_str).collect();
        let with_p = |verb: &str, extra: &[&str]| {
            let mut argv = vec!["cargo", verb];
            for p in &packages {
                argv.extend(["-p", p]);
            }
            argv.extend(extra);
            job(format!("{verb} {}", packages.join(" ")), &argv)
        };
        out.push(job("fmt", &["cargo", "fmt", "--all", "--", "--check"]));
        out.push(with_p(
            "clippy",
            &["--all-targets", "--locked", "--", "-D", "warnings"],
        ));
        if affected.contains(&layout.cli) {
            let cli_dir = crates.cli_dir().unwrap_or_default();
            let unit_crate = |paths: &[String]| {
                crates
                    .by_dir
                    .iter()
                    .find(|(dir, _)| paths.iter().any(|p| p == *dir))
                    .map(|(_, c)| c.clone())
            };
            let mut features: BTreeSet<String> = config
                .units
                .values()
                .filter(|u| !u.features.is_empty())
                .filter(|u| {
                    unit_crate(&u.paths).is_some_and(|c| affected.contains(&c))
                        || files.iter().any(|f| {
                            !cli_dir.is_empty()
                                && f.starts_with(cli_dir)
                                && u.paths.iter().any(|p| f.starts_with(p.as_str()))
                        })
                })
                .map(|u| u.features.clone())
                .collect();
            features.extend(crates.pigeon_features_touched(&affected, files));
            out.extend(features.iter().map(|f| feature_clippy(&layout.cli, f)));
        }
        out.extend(
            crates
                .workers()
                .filter(|w| touched(w) || affected.contains(&w.package))
                .map(worker_job),
        );
        if let Some(picked) = map.and_then(|m| m.select(crates, files)) {
            out.extend(picked);
        } else {
            out.push(Job {
                units: packages.iter().map(|p| Unit::whole(p)).collect(),
                ..with_p("test", &["--locked"])
            });
        }
    }
    out
}

/// How much of the full set the merge gate of a change touching `files` runs: full for a
/// workspace-wide path, or tests the map chose across more than [`MAX_PACKAGES`] packages;
/// scoped otherwise, since tests chosen by crate cover every package the change reaches. See
/// `.claude/decisions/2026-11-15-development-code-checks-by-reach.md` §3.
#[must_use]
pub fn gate_of(crates: &Crates, files: &[String], map: Option<&TestMap>) -> Gate {
    let files: Vec<&str> = files
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .collect();
    let everything = crates.layout.everything();
    if let Some(f) = files
        .iter()
        .find(|f| everything.iter().any(|e| f.starts_with(e.as_str())))
    {
        return Gate::Full(format!("{f} runs every check"));
    }
    match map.and_then(|m| m.select(crates, &files)) {
        Some(picked) if picked.len() > MAX_PACKAGES => {
            Gate::Full(format!("its tests span {} packages", picked.len()))
        }
        _ => Gate::Scoped,
    }
}

/// `cargo test --workspace`, the whole suite, running every package.
fn workspace_tests(crates: &Crates) -> Job {
    Job {
        units: crates.by_dir.values().map(|p| Unit::whole(p)).collect(),
        ..job("test", &["cargo", "test", "--workspace", "--locked"])
    }
}

/// The merge gate set of a head whose checks are `checks`, as the bound job's `full_set` builds
/// it: its code checks run on GitHub, and the whole suite.
fn gate_set(crates: &Crates, checks: &[Job]) -> Vec<Job> {
    let mut full: Vec<Job> = checks
        .iter()
        .filter(|c| crate::actions::on_github(&c.argv))
        .cloned()
        .collect();
    let suite = workspace_tests(crates);
    if !full.is_empty() && !full.iter().any(|c| c.argv == suite.argv) {
        full.push(suite);
    }
    full
}

/// File names whose change on `main` reaches every check, wherever they are; see the merge-path
/// ADR's 2026-11-16 amendment (#20).
const REACH_ALL: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "build.rs",
];

/// What `main`'s changed `files` reach of a head whose checks are `checks`: none means every
/// check. A `.rs` file in a crate reaches tests by the map; any other file reaches every check
/// when its own selection names one of `checks` but `policy.check`. See
/// `.claude/decisions/2026-11-16-development-merge-path-measured-faster.md`'s 2026-11-16
/// amendment (#20).
fn reach_of(
    config: &Config,
    crates: &Crates,
    map: Option<&TestMap>,
    files: &[String],
    checks: &[Job],
) -> Option<map::Reached> {
    let map = map?;
    let own: BTreeSet<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    let mut rust = Vec::new();
    for f in files.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
        let name = f.rsplit('/').next().unwrap_or(f);
        if REACH_ALL.contains(&name) {
            return None;
        }
        let in_crate = crates.by_dir.keys().any(|d| f.starts_with(d.as_str()));
        if in_crate
            && std::path::Path::new(f)
                .extension()
                .is_some_and(|e| e == "rs")
        {
            rust.push(f);
            continue;
        }
        let picked = jobs_of(config, crates, &[f.to_string()], Some(map));
        if picked
            .iter()
            .any(|j| j.name != "policy.check" && own.contains(j.name.as_str()))
        {
            return None;
        }
    }
    Some(map.reach(crates, &rust))
}

/// Whether `reach` reaches `job`: every job when it is none; else a `cargo test` job that runs a
/// unit it reaches, or records no units.
fn reaches(reach: Option<&map::Reached>, job: &Job) -> bool {
    let Some(r) = reach else {
        return true;
    };
    let test = job.argv.first().is_some_and(|a| a == "cargo")
        && job.argv.get(1).is_some_and(|a| a == "test");
    test && (job.units.is_empty() || r.hits(&job.units))
}

/// The checks among `checks` that `main`'s changed `files` reach, by name; with no `map`, every
/// one.
#[must_use]
pub fn reached_of(
    config: &Config,
    crates: &Crates,
    map: Option<&TestMap>,
    files: &[String],
    checks: &[Job],
) -> BTreeSet<String> {
    let reach = reach_of(config, crates, map, files, checks);
    checks
        .iter()
        .filter(|c| reaches(reach.as_ref(), c))
        .map(|c| c.name.clone())
        .collect()
}

/// The names among `kept`, a head's passed checks and gate jobs, that carry to its restacked
/// head touching `head`: those the restacked head's own checks or gate set hold with the same
/// name and argv, and that `main`'s changed `files` don't reach. See
/// `.claude/decisions/2026-11-16-development-merge-path-measured-faster.md`'s 2026-11-16
/// amendment (#20).
#[must_use]
pub fn carried_of(
    config: &Config,
    crates: &Crates,
    map: Option<&TestMap>,
    files: &[String],
    head: &[String],
    kept: &[crate::change::Check],
) -> BTreeSet<String> {
    let checks: Vec<Job> = jobs_of(config, crates, head, map)
        .into_iter()
        .filter(|j| j.name != "policy.check")
        .collect();
    let full = gate_set(crates, &checks);
    let mut all = checks.clone();
    all.extend(full.iter().cloned());
    let reach = reach_of(config, crates, map, files, &all);
    kept.iter()
        .filter(|k| {
            let (name, pool) = match k.name.strip_prefix(crate::change::GATE) {
                Some(n) => (n, &full),
                None => (k.name.as_str(), &checks),
            };
            pool.iter()
                .any(|j| j.name == name && j.argv == k.argv && !reaches(reach.as_ref(), j))
        })
        .map(|k| k.name.clone())
        .collect()
}

/// `pigeon`'s features, from the release units and its manifest.
fn all_features(config: &Config, crates: &Crates) -> BTreeSet<String> {
    config
        .units
        .values()
        .filter(|u| !u.features.is_empty())
        .map(|u| u.features.clone())
        .chain(crates.features.keys().cloned())
        .collect()
}

/// Every check a workspace-wide change runs, from the workspace's `crates` and release `config`:
/// the full set the merge gate runs on a head; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §5.
#[must_use]
pub fn full_of(config: &Config, crates: &Crates) -> Vec<Job> {
    everything(&all_features(config, crates), crates)
}

/// Whether `out` already tests all of `package`, alone or as the whole workspace.
fn ran_tests_of(out: &[Job], package: &str) -> bool {
    out.iter().any(|j| {
        j.argv.get(1).is_some_and(|v| v == "test")
            && !j
                .argv
                .iter()
                .any(|a| matches!(a.as_str(), "--lib" | "--bin" | "--test"))
            && (j.argv.iter().any(|a| a == "--workspace")
                || j.argv.windows(2).any(|w| w[0] == "-p" && w[1] == package))
    })
}

/// The checks a change touching `files` needs, in the order to run them, from the workspace's
/// `crates` and release `config`, with the tests a Rust change runs chosen by `map` when it knows
/// the change; `policy.check` always runs, and a workspace-wide setting runs everything.
#[must_use]
pub fn jobs_of(
    config: &Config,
    crates: &Crates,
    files: &[String],
    map: Option<&TestMap>,
) -> Vec<Job> {
    let files: Vec<&str> = files
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .collect();
    let all_features = all_features(config, crates);
    let layout = &crates.layout;
    let wide = layout.everything();
    let mut out = Vec::new();
    if files
        .iter()
        .any(|f| wide.iter().any(|e| f.starts_with(e.as_str())))
    {
        out.extend(everything(&all_features, crates));
    } else {
        out.extend(rust_jobs(crates, config, &files, map));
    }
    if files.iter().any(|f| under(f, &INFRA)) {
        out.push(Job {
            name: "infra.validate".into(),
            argv: layout.operation("infra.validate"),
            units: Vec::new(),
        });
        if let Some(t) = &layout.infra_tests
            && !ran_tests_of(&out, &t.package)
        {
            let modules: Vec<&str> = t.modules.iter().map(|m| m.trim_end_matches("::")).collect();
            // Before its suite exists, each module is its own test target.
            let own_targets = crates.tests.get(&t.package).is_some_and(|known| {
                !known.contains(&t.test) && modules.iter().all(|m| known.contains(*m))
            });
            let mut argv = vec!["cargo", "test", "-p", t.package.as_str()];
            let (name, units) = if own_targets {
                argv.push("--locked");
                for m in &modules {
                    argv.extend(["--test", m]);
                }
                (
                    format!("test {}: {}", t.package, modules.join(" ")),
                    modules
                        .iter()
                        .map(|m| Unit::target(&t.package, "test", m, ""))
                        .collect(),
                )
            } else {
                argv.extend(["--test", t.test.as_str(), "--locked", "--"]);
                argv.extend(t.modules.iter().map(String::as_str));
                (
                    format!("test {}: {} {}", t.package, t.test, modules.join(" ")),
                    t.modules
                        .iter()
                        .map(|m| Unit::target(&t.package, "test", &t.test, m))
                        .collect(),
                )
            };
            out.push(Job {
                units,
                ..job(name, &argv)
            });
        }
    }
    let policy_tests: Vec<&str> = crates.of(&layout
        .policy_tests
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>());
    if files.iter().any(|f| under(f, &POLICIES))
        && !policy_tests.is_empty()
        && !policy_tests.iter().all(|p| ran_tests_of(&out, p))
    {
        let mut argv = vec!["cargo", "test"];
        for p in &policy_tests {
            argv.extend(["-p", p]);
        }
        argv.push("--locked");
        out.push(Job {
            units: policy_tests.iter().map(|p| Unit::whole(p)).collect(),
            ..job("test this repository's policies", &argv)
        });
    }
    if files.iter().any(|f| under(f, &SITES)) {
        out.push(Job {
            name: "sites.build".into(),
            argv: layout.operation("sites.build"),
            units: Vec::new(),
        });
    }
    if files.iter().any(|f| under(f, &IOS)) {
        out.push(job("ios", &["bash", crate::actions::IOS_CHECK]));
    }
    out.extend(spikes(layout, &files));
    if files.contains(&TEST_MAP) {
        out.push(test_map());
    }
    out.push(Job {
        name: "policy.check".into(),
        argv: layout.operation("policy.check"),
        units: Vec::new(),
    });
    out
}

/// The test map's script: each test binary and the workspace files it runs; see the 2026-11-11
/// amendment to `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
pub const TEST_MAP: &str = "tooling/ci/test-map.sh";

/// The `test map` check, run on GitHub from the workspace; it writes `tests.json`.
#[must_use]
pub fn test_map() -> Job {
    Job {
        name: "test map".into(),
        argv: vec!["bash".into(), format!("../../{TEST_MAP}")],
        units: Vec::new(),
    }
}

/// Where spikes live: `tooling/spikes/<name>/run.sh`, run as a check with no credentials; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §4.
pub const SPIKES: &str = "tooling/spikes/";

/// One `spike <name>` check per spike a change touches, running its `run.sh` from the workspace.
fn spikes(layout: &Layout, files: &[&str]) -> Vec<Job> {
    let names: BTreeSet<&str> = files
        .iter()
        .filter_map(|f| f.strip_prefix(SPIKES)?.split_once('/'))
        .map(|(name, _)| name)
        .filter(|n| {
            !n.is_empty()
                && n.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .collect();
    names
        .into_iter()
        .map(|n| Job {
            name: format!("spike {n}"),
            argv: vec![
                "sh".into(),
                layout.from_workspace(&format!("{SPIKES}{n}/run.sh")),
            ],
            units: Vec::new(),
        })
        .collect()
}

/// Paths whose change makes the job image too old for the Worker: the image's tools and its
/// entrypoint. A Worker
/// deploy waits for a new image; jobs run `main`'s newest `pigeon` from `builds/`, so its code
/// is not among them; see `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md`
/// §3.
pub const IMAGE_BLOCKS: [&str; 4] = [
    "mise.toml",
    "Dockerfile",
    "pigeon-start",
    "rust-toolchain.toml",
];

/// `pigeon`'s crates, every workspace member it builds from, and the root's manifest and lockfile.
const RUST: [&str; 6] = [
    "src/common/",
    "src/pigeon-core/",
    "src/pigeon-eviction/",
    "src/pigeon-agent/",
    "src/pigeon-harness/",
    "Cargo.",
];

/// The judge's crate, and the root's manifest and lockfile; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §B.
const JUDGE: [&str; 2] = ["src/pigeon-eviction/", "Cargo."];

/// The files among `files` that make the job image block a Worker deploy.
#[must_use]
pub fn image_inputs(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| IMAGE_BLOCKS.contains(&f.as_str()))
        .cloned()
        .collect()
}

/// Whether a deploy of `main` touching `files` builds `pigeon` again: when any of [`RUST`] changed; `files` of `None` means everything.
#[must_use]
pub fn builds(files: Option<&[String]>) -> bool {
    files.is_none_or(|f| f.iter().any(|f| under(f, &RUST)))
}

/// Whether a deploy of `main` touching `files` builds the judge again: when [`JUDGE`] changed; `files` of `None` means everything.
#[must_use]
pub fn builds_judge(files: Option<&[String]>) -> bool {
    files.is_none_or(|f| f.iter().any(|f| under(f, &JUDGE)))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Crates, Layout, everything};

    #[test]
    fn a_workspace_setting_builds_only_the_workers_the_tree_has() {
        let crates = Crates {
            by_dir: [("src/pigeon-agent/".to_string(), "pigeon-agent".to_string())].into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
            tests: BTreeMap::default(),
            bins: BTreeMap::default(),
            layout: Layout {
                worker: Some(super::Worker {
                    package: "pigeon-agent".into(),
                    name: "clippy agent worker (wasm)".into(),
                    only: vec!["src/pigeon-agent/src/worker/".into()],
                }),
                workers: Vec::new(),
                ..Layout::new("pigeon")
            },
        };
        assert_eq!(crates.of(&["pigeon-agent"]), ["pigeon-agent"]);
        let names: Vec<String> = everything(&BTreeSet::default(), &crates)
            .into_iter()
            .map(|j| j.name)
            .collect();
        assert!(
            names.contains(&"clippy agent worker (wasm)".to_string()),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("--features")), "{names:?}");
    }

    #[test]
    fn a_gate_is_full_only_for_a_workspace_setting_or_mapped_tests_across_packages() {
        use super::{Binary, Gate, TestMap, gate_of};
        let dir = |p: &str| format!("src/pigeon-cli/crates/{p}/");
        let crates = Crates {
            by_dir: [
                (dir("agent"), "pigeon-agent".to_string()),
                (dir("pigeon"), "pigeon".to_string()),
                (dir("eviction"), "pigeon-eviction".to_string()),
            ]
            .into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
            tests: BTreeMap::default(),
            bins: BTreeMap::default(),
            layout: Layout::new("pigeon"),
        };
        let bin = |package: &str, name: &str, file: &str| Binary {
            package: package.into(),
            kind: "test".into(),
            name: name.into(),
            filter: String::new(),
            files: vec![file.into()],
        };
        let a = "src/pigeon-cli/crates/agent/src/a.rs";
        let b = "src/pigeon-cli/crates/pigeon/src/b.rs";
        let c = "src/pigeon-cli/crates/eviction/src/c.rs";
        let map = TestMap {
            sha: "x".into(),
            binaries: vec![
                bin("pigeon-agent", "a", a),
                bin("pigeon", "b", b),
                bin("pigeon-eviction", "c", c),
                bin("pigeon-agent", "d", "elsewhere.rs"),
                bin("pigeon", "e", "elsewhere.rs"),
                bin("pigeon-eviction", "f", "elsewhere.rs"),
                bin("pigeon-agent", "g", "elsewhere.rs"),
            ],
        };
        let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let scoped = |g: &Gate| *g == Gate::Scoped;
        assert!(scoped(&gate_of(&crates, &files(&[a]), Some(&map))));
        assert!(
            scoped(&gate_of(&crates, &files(&[a, b]), Some(&map))),
            "2 packages"
        );
        for (name, f, m) in [
            ("no Rust", files(&["docs/x.md"]), None),
            ("no map: tests by crate", files(&[a]), None),
            (
                "unknown to the map: tests by crate",
                files(&["src/pigeon-cli/crates/pigeon/src/new.rs"]),
                Some(&map),
            ),
            (
                "wasm-only",
                files(&["src/pigeon-cli/crates/agent/src/worker/x.rs"]),
                Some(&map),
            ),
        ] {
            assert!(scoped(&gate_of(&crates, &f, m)), "{name}");
        }
        for (name, f, m) in [
            ("3 packages", files(&[a, b, c]), Some(&map)),
            ("workspace-wide", files(&[a, "mise.toml"]), Some(&map)),
        ] {
            assert!(!scoped(&gate_of(&crates, &f, m)), "{name}");
        }
    }

    fn root_layout() -> Crates {
        let layout = Layout {
            workspace: String::new(),
            cli: "pigeon-harness".into(),
            worker: Some(super::Worker {
                package: "pigeon-agent".into(),
                name: "clippy agent worker (wasm)".into(),
                only: vec!["src/pigeon-agent/src/worker/".into()],
            }),
            workers: Vec::new(),
            policy_tests: vec!["pigeon-eviction".into(), "pigeon-gone".into()],
            infra_tests: None,
            run: vec!["run".into()],
            instance: vec!["instance".into()],
            ..Layout::new("pigeon-harness")
        };
        Crates {
            by_dir: [
                ("src/pigeon-agent/".to_string(), "pigeon-agent".to_string()),
                (
                    "src/pigeon-harness/".to_string(),
                    "pigeon-harness".to_string(),
                ),
                (
                    "src/pigeon-eviction/".to_string(),
                    "pigeon-eviction".to_string(),
                ),
            ]
            .into(),
            deps: [(
                "pigeon-harness".to_string(),
                ["pigeon-agent".to_string()].into(),
            )]
            .into(),
            features: [("infra".to_string(), ["pigeon-agent".to_string()].into())].into(),
            tests: BTreeMap::default(),
            bins: BTreeMap::default(),
            layout,
        }
    }

    #[test]
    fn a_layout_needs_its_cli_and_names_no_crate_by_default() {
        assert!(
            toml::from_str::<Layout>("run = [\"run\"]\n").is_err(),
            "no cli"
        );
        let read: Layout = toml::from_str("cli = \"x\"\n").expect("parses");
        assert_eq!(read, Layout::new("x"));
        assert_eq!((read.worker, read.infra_tests), (None, None));
        assert_eq!(read.policy_tests, Vec::<String>::new());
        let nested = Layout {
            workspace: "ws/a".into(),
            ..Layout::new("x")
        };
        assert_eq!(nested.root(), "../..");
        assert_eq!(nested.from_workspace("tooling/x.sh"), "../../tooling/x.sh");
        assert_eq!(nested.in_workspace("Cargo.toml"), "ws/a/Cargo.toml");
        assert_eq!(
            nested.everything(),
            [
                "ws/a/Cargo.toml",
                "ws/a/Cargo.lock",
                "ws/a/.cargo/",
                "mise.toml",
                super::RELEASES,
            ]
        );
    }

    #[test]
    fn a_layout_at_the_root_runs_its_checks_from_the_root() {
        let crates = root_layout();
        let l = &crates.layout;
        assert_eq!(l.root(), ".");
        assert_eq!(l.from_workspace("tooling/x.sh"), "tooling/x.sh");
        assert_eq!(l.everything()[..2], ["Cargo.toml", "Cargo.lock"]);
        let config = crate::release::Config::parse(
            "description: t\nrepository: https://example.test/r\nunits: {}\ntiers: []\n",
        )
        .expect("config");
        let jobs = super::jobs_of(
            &config,
            &crates,
            &[
                "src/pigeon-agent/src/worker/x.rs".to_string(),
                "src/pigeon-agent/src/lib.rs".to_string(),
                ".config/policies/a.yaml".to_string(),
                "tooling/spikes/s/run.sh".to_string(),
            ],
            None,
        );
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        assert!(names.contains(&"clippy agent worker (wasm)"), "{names:?}");
        assert!(
            names.contains(&"clippy pigeon-harness --features infra"),
            "{names:?}"
        );
        let argv = |name: &str| {
            jobs.iter()
                .find(|j| j.name == name)
                .map(|j| j.argv.join(" "))
                .unwrap_or_default()
        };
        assert_eq!(
            argv("policy.check"),
            "cargo run -q --locked -p pigeon-harness -- run policy.check --root ."
        );
        assert_eq!(argv("spike s"), "sh tooling/spikes/s/run.sh");
        assert_eq!(
            argv("test this repository's policies"),
            "cargo test -p pigeon-eviction --locked"
        );
        let wide = super::jobs_of(&config, &crates, &["Cargo.lock".to_string()], None);
        assert_eq!(
            wide.last().map(|j| j.argv.join(" ")),
            Some("cargo run -q --locked -p pigeon-harness -- run policy.check --root .".into())
        );
        assert!(
            wide.iter().any(|j| j.argv.join(" ")
                == "cargo run -q --locked -p pigeon-harness -- instance --root .")
        );
    }

    #[test]
    fn a_deploy_builds_and_waits_for_an_image_in_either_layout() {
        let f = |p: &str| vec![p.to_string()];
        for p in [
            "src/pigeon-core/src/a.rs",
            "src/pigeon-harness/src/a.rs",
            "Cargo.lock",
        ] {
            assert!(super::builds(Some(&f(p))), "{p}");
        }
        for p in ["src/pigeon-eviction/src/a.rs", "Cargo.toml"] {
            assert!(super::builds_judge(Some(&f(p))), "{p}");
        }
        for p in ["src/pigeon-core/src/a.rs", "src/pigeon-harness/src/a.rs"] {
            assert!(!super::builds_judge(Some(&f(p))), "{p}");
        }
        for p in [
            "src/pigeon-ios/a.swift",
            "docs/Cargo.md",
            "src/pigeon-cli/crates/x/a.rs",
        ] {
            assert!(!super::builds(Some(&f(p))), "{p}");
        }
        assert_eq!(super::image_inputs(&f("Dockerfile")), ["Dockerfile"]);
        assert_eq!(
            super::image_inputs(&f("src/pigeon-cli/Dockerfile")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_mapped_target_that_is_gone_runs_as_its_suite_module() {
        use super::{Binary, TestMap};
        let crates = Crates {
            by_dir: [
                (
                    "src/pigeon-cli/crates/pigeon/".to_string(),
                    "pigeon".to_string(),
                ),
                (
                    "src/pigeon-cli/crates/eviction/".to_string(),
                    "pigeon-eviction".to_string(),
                ),
            ]
            .into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
            tests: [
                ("pigeon".to_string(), BTreeSet::from(["suite".to_string()])),
                (
                    "pigeon-eviction".to_string(),
                    BTreeSet::from(["cli".to_string()]),
                ),
            ]
            .into(),
            bins: BTreeMap::default(),
            layout: Layout::new("pigeon"),
        };
        let unit = |package: &str, name: &str, file: &str| Binary {
            package: package.into(),
            kind: "test".into(),
            name: name.into(),
            filter: String::new(),
            files: vec![file.into()],
        };
        let a = "src/pigeon-cli/crates/pigeon/src/a.rs";
        let b = "src/pigeon-cli/crates/eviction/src/b.rs";
        let map = TestMap {
            sha: "x".into(),
            binaries: vec![
                unit("pigeon", "agent", a),
                unit("pigeon-eviction", "tree", b),
                unit("pigeon", "e1", "elsewhere.rs"),
                unit("pigeon", "e2", "elsewhere.rs"),
                unit("pigeon", "e3", "elsewhere.rs"),
            ],
        };
        let jobs = map.select(&crates, &[a]).expect("chosen");
        assert_eq!(
            jobs.iter().map(|j| j.argv.join(" ")).collect::<Vec<_>>(),
            ["cargo test -p pigeon --test suite --locked -- agent::"],
            "{jobs:?}"
        );
        assert!(
            map.select(&crates, &[b]).is_none(),
            "a target that is gone, with no suite, leaves it to the crate"
        );
    }

    #[test]
    fn a_mapped_package_that_was_renamed_leaves_it_to_the_crate() {
        use super::{Binary, TestMap};
        let crates = Crates {
            by_dir: [("src/pigeon-agent/".to_string(), "pigeon-agent".to_string())].into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
            tests: [(
                "pigeon-agent".to_string(),
                BTreeSet::from(["suite".to_string()]),
            )]
            .into(),
            bins: BTreeMap::default(),
            layout: Layout::new("pigeon"),
        };
        let unit = |package: &str, file: &str| Binary {
            package: package.into(),
            kind: "lib".into(),
            name: "agent".into(),
            filter: String::new(),
            files: vec![file.into()],
        };
        let a = "src/pigeon-agent/src/a.rs";
        let map = TestMap {
            sha: "x".into(),
            binaries: vec![
                unit("pigeon-retired", a),
                unit("pigeon-retired", "e1.rs"),
                unit("pigeon-retired", "e2.rs"),
                unit("pigeon-retired", "e3.rs"),
            ],
        };
        assert!(
            map.select(&crates, &[a]).is_none(),
            "a unit of a package the workspace no longer has can't say"
        );
    }

    #[test]
    fn a_mapped_binary_that_was_renamed_leaves_it_to_the_crate() {
        use super::{Binary, TestMap};
        let crates = Crates {
            by_dir: [("src/judge/".to_string(), "judge".to_string())].into(),
            bins: [(
                "judge".to_string(),
                BTreeSet::from(["judge-new".to_string()]),
            )]
            .into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
            tests: BTreeMap::default(),
            layout: Layout::new("pigeon"),
        };
        let unit = |kind: &str, name: &str, file: &str| Binary {
            package: "judge".into(),
            kind: kind.into(),
            name: name.into(),
            filter: String::new(),
            files: vec![file.into()],
        };
        let a = "src/judge/src/main.rs";
        let b = "src/judge/src/rules.rs";
        let map = TestMap {
            sha: "x".into(),
            binaries: vec![
                unit("bin", "judge-old", a),
                unit("lib", "judge", b),
                unit("lib", "judge", "e1.rs"),
                unit("lib", "judge", "e2.rs"),
                unit("lib", "judge", "e3.rs"),
            ],
        };
        assert!(
            map.select(&crates, &[a]).is_none(),
            "a binary the package no longer builds can't say"
        );
        let jobs = map.select(&crates, &[b]).expect("chosen");
        assert_eq!(
            jobs.iter().map(|j| j.argv.join(" ")).collect::<Vec<_>>(),
            ["cargo test -p judge --lib --locked"]
        );
    }
}
