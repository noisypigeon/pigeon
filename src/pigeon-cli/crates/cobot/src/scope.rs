//! The checks a change needs, from the paths it touches; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::Error;
use crate::release::Config;

/// The Rust workspace, from the root.
pub const WORKSPACE: &str = "src/pigeon-cli";

/// The release configuration, from the root.
pub const RELEASES: &str = ".config/releases.yaml";

/// Paths whose change runs every check.
const EVERYTHING: [&str; 5] = [
    "src/pigeon-cli/Cargo.toml",
    "src/pigeon-cli/Cargo.lock",
    "src/pigeon-cli/.cargo/",
    "mise.toml",
    RELEASES,
];

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
const SITES: [&str; 4] = ["src/site-templates/", "workloads/", "sites/", "config.yaml"];

/// The crates whose `worker` build compiles to wasm.
const WORKER: [&str; 2] = ["cobot", "pigeon-backplane"];

/// One check: a name, and the command to run in [`WORKSPACE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    /// What it checks.
    pub name: String,
    /// The command, run in [`WORKSPACE`].
    pub argv: Vec<String>,
}

fn job(name: impl Into<String>, argv: &[&str]) -> Job {
    Job {
        name: name.into(),
        argv: argv.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn pigeon(args: &[&str]) -> Vec<String> {
    ["cargo", "run", "-q", "--locked", "-p", "pigeon", "--"]
        .iter()
        .chain(args)
        .map(|s| (*s).to_string())
        .collect()
}

/// The workspace's crates: each crate directory's package, and each package's workspace
/// dependencies.
struct Crates {
    by_dir: BTreeMap<String, String>,
    deps: BTreeMap<String, BTreeSet<String>>,
    /// `pigeon`'s features, each with the packages it enables.
    features: BTreeMap<String, BTreeSet<String>>,
}

fn read_toml(path: &Path) -> Result<toml::Value, Error> {
    let io = |message: String| Error::Io {
        path: path.to_path_buf(),
        message,
    };
    fs::read_to_string(path)
        .map_err(|e| io(e.to_string()))?
        .parse()
        .map_err(|e: toml::de::Error| io(e.to_string()))
}

fn crates(root: &Path) -> Result<Crates, Error> {
    let ws = root.join(WORKSPACE);
    let workspace = read_toml(&ws.join("Cargo.toml"))?;
    let members: BTreeMap<String, String> = workspace["workspace"]["dependencies"]
        .as_table()
        .into_iter()
        .flatten()
        .filter_map(|(key, v)| {
            let path = v.get("path")?.as_str()?;
            let package = v
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            Some((key.clone(), format!("{path}/{package}")))
        })
        .collect();
    let mut by_dir = BTreeMap::new();
    let mut deps = BTreeMap::new();
    let mut features = BTreeMap::new();
    for entry in fs::read_dir(ws.join("crates"))
        .map_err(|e| Error::Io {
            path: ws.join("crates"),
            message: e.to_string(),
        })?
        .flatten()
    {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let m = read_toml(&manifest)?;
        let Some(name) = m["package"]["name"].as_str() else {
            continue;
        };
        let dir = entry.file_name().to_string_lossy().into_owned();
        let mine: BTreeSet<String> = ["dependencies", "dev-dependencies"]
            .iter()
            .filter_map(|t| m.get(*t).and_then(toml::Value::as_table))
            .flat_map(|t| t.keys())
            .filter_map(|k| members.get(k))
            .filter_map(|p| p.rsplit('/').next().map(str::to_string))
            .collect();
        if name == "pigeon" {
            features = pigeon_features(&m, &members);
        }
        by_dir.insert(format!("{WORKSPACE}/crates/{dir}/"), name.to_string());
        deps.insert(name.to_string(), mine);
    }
    Ok(Crates {
        by_dir,
        deps,
        features,
    })
}

/// `pigeon`'s features but `default`, each with the workspace packages its `dep:` entries enable.
fn pigeon_features(
    manifest: &toml::Value,
    members: &BTreeMap<String, String>,
) -> BTreeMap<String, BTreeSet<String>> {
    manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .filter(|(f, _)| f.as_str() != "default")
        .map(|(f, list)| {
            let packages = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str()?.strip_prefix("dep:"))
                .filter_map(|k| members.get(k))
                .filter_map(|p| p.rsplit('/').next().map(str::to_string))
                .collect();
            (f.clone(), packages)
        })
        .collect()
}

/// `pigeon`'s files every command shares.
const PIGEON_SHARED: [&str; 5] = [
    "src/main.rs",
    "src/lib.rs",
    "src/cli/mod.rs",
    "src/commands/mod.rs",
    "Cargo.toml",
];

impl Crates {
    /// The `pigeon` features whose build `files` and the `affected` packages change: a feature
    /// whose package is affected, whose `commands/<f>.rs` or `cli/<f>.rs` changed, or every one
    /// when a file all commands share changed.
    fn pigeon_features_touched(
        &self,
        affected: &BTreeSet<String>,
        files: &[&str],
    ) -> BTreeSet<String> {
        let prefix = format!("{WORKSPACE}/crates/pigeon/");
        let pigeon: Vec<&str> = files
            .iter()
            .filter_map(|f| f.strip_prefix(&prefix))
            .collect();
        if pigeon.iter().any(|f| PIGEON_SHARED.contains(f)) {
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
fn everything(features: &BTreeSet<String>) -> Vec<Job> {
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
    out.push(job(
        "clippy pigeon (no features)",
        &[
            "cargo",
            "clippy",
            "-p",
            "pigeon",
            "--all-targets",
            "--locked",
            "--no-default-features",
            "--",
            "-D",
            "warnings",
        ],
    ));
    out.extend(features.iter().map(feature_clippy));
    out.extend(worker_jobs(&WORKER));
    out.push(job("test", &["cargo", "test", "--workspace", "--locked"]));
    out.push(Job {
        name: "backplane instance".into(),
        argv: pigeon(&["backplane", "instance", "--root", "../.."]),
    });
    out
}

fn feature_clippy(f: &String) -> Job {
    job(
        format!("clippy pigeon --features {f}"),
        &[
            "cargo",
            "clippy",
            "-p",
            "pigeon",
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

/// The `worker` builds' checks for `crates`, among [`WORKER`].
fn worker_jobs(crates: &[&str]) -> Vec<Job> {
    let clippy = |name: &str, package: &str, wasm: bool| {
        let mut argv = vec![
            "cargo",
            "clippy",
            "-p",
            package,
            "--lib",
            "--locked",
            "--no-default-features",
            "--features",
            "worker",
        ];
        if wasm {
            argv.extend(["--target", "wasm32-unknown-unknown"]);
        }
        argv.extend(["--", "-D", "warnings"]);
        job(name, &argv)
    };
    let mut out = Vec::new();
    if crates.contains(&"cobot") {
        out.push(clippy("clippy cobot worker", "cobot", false));
        out.push(clippy("clippy cobot worker (wasm)", "cobot", true));
        out.push(job(
            "test cobot worker",
            &[
                "cargo",
                "test",
                "-p",
                "cobot",
                "--lib",
                "--locked",
                "--features",
                "worker",
            ],
        ));
    }
    if crates.contains(&"pigeon-backplane") {
        out.push(clippy(
            "clippy backplane worker (wasm)",
            "pigeon-backplane",
            true,
        ));
    }
    out
}

/// The Rust checks for the crates `files` touch and their dependants.
fn rust_jobs(crates: &Crates, config: &Config, files: &[&str]) -> Vec<Job> {
    let mut out = Vec::new();
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
        if affected.contains("pigeon") {
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
                            f.starts_with(&format!("{WORKSPACE}/crates/pigeon/"))
                                && u.paths.iter().any(|p| f.starts_with(p.as_str()))
                        })
                })
                .map(|u| u.features.clone())
                .collect();
            features.extend(crates.pigeon_features_touched(&affected, files));
            out.extend(features.iter().map(feature_clippy));
        }
        let workers: Vec<&str> = WORKER
            .iter()
            .copied()
            .filter(|w| affected.contains(*w))
            .collect();
        out.extend(worker_jobs(&workers));
        out.push(with_p("test", &["--locked"]));
    }
    out
}

/// Whether `out` already tests `package`, alone or as the whole workspace.
fn ran_tests_of(out: &[Job], package: &str) -> bool {
    out.iter().any(|j| {
        j.argv.get(1).is_some_and(|v| v == "test")
            && (j.argv.iter().any(|a| a == "--workspace")
                || j.argv.windows(2).any(|w| w[0] == "-p" && w[1] == package))
    })
}

/// The checks a change touching `files` needs, in the order to run them; `policy.check`
/// always runs, and a workspace-wide setting runs everything.
///
/// # Errors
///
/// [`Error::Io`] when the workspace's manifests or the release configuration can't be read.
pub fn jobs(root: &Path, files: &[String]) -> Result<Vec<Job>, Error> {
    let files: Vec<&str> = files
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .collect();
    let config = Config::load(&root.join(RELEASES))?;
    let crates = crates(root)?;
    let all_features: BTreeSet<String> = config
        .units
        .values()
        .filter(|u| !u.features.is_empty())
        .map(|u| u.features.clone())
        .chain(crates.features.keys().cloned())
        .collect();
    let mut out = Vec::new();
    if files.iter().any(|f| under(f, &EVERYTHING)) {
        out.extend(everything(&all_features));
    } else {
        out.extend(rust_jobs(&crates, &config, &files));
    }
    let ran_cobot = ran_tests_of(&out, "cobot");
    if files.iter().any(|f| under(f, &INFRA)) {
        out.push(Job {
            name: "infra.validate".into(),
            argv: pigeon(&["backplane", "run", "infra.validate", "--root", "../.."]),
        });
        if !ran_cobot {
            out.push(job(
                "test cobot (catalog, manifest, apex, git, credentials)",
                &[
                    "cargo",
                    "test",
                    "-p",
                    "cobot",
                    "--locked",
                    "--test",
                    "catalog",
                    "--test",
                    "manifest",
                    "--test",
                    "apex",
                    "--test",
                    "git",
                    "--test",
                    "credentials",
                ],
            ));
        }
    }
    let policy_tests = ["pigeon-sentinel", "pigeon", "pigeon-backplane"];
    if files.iter().any(|f| under(f, &POLICIES))
        && !policy_tests.iter().all(|p| ran_tests_of(&out, p))
    {
        out.push(job(
            "test this repository's policies",
            &[
                "cargo",
                "test",
                "-p",
                "pigeon-sentinel",
                "-p",
                "pigeon",
                "-p",
                "pigeon-backplane",
                "--locked",
            ],
        ));
    }
    if files.iter().any(|f| under(f, &SITES)) {
        out.push(Job {
            name: "sites.build".into(),
            argv: pigeon(&["backplane", "run", "sites.build", "--root", "../.."]),
        });
    }
    out.extend(spikes(&files));
    out.push(Job {
        name: "policy.check".into(),
        argv: pigeon(&["backplane", "run", "policy.check", "--root", "../.."]),
    });
    Ok(out)
}

/// Where spikes live: `tooling/spikes/<name>/run.sh`, run as a check with no credentials; see
/// `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md` §4.
pub const SPIKES: &str = "tooling/spikes/";

/// One `spike <name>` check per spike a change touches, running its `run.sh` from the workspace.
fn spikes(files: &[&str]) -> Vec<Job> {
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
            argv: vec!["sh".into(), format!("../../{SPIKES}{n}/run.sh")],
        })
        .collect()
}

fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>, Error> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| Error::Tool {
            command: "git".into(),
            message: e.to_string(),
        })?;
    if !out.status.success() {
        return Err(Error::Tool {
            command: format!("git {}", args.join(" ")),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// The paths a change touches: committed since it left `base`, plus uncommitted and untracked
/// ones, sorted and once each.
///
/// # Errors
///
/// [`Error::Tool`] when git fails, e.g. on an unknown `base`.
pub fn changed(root: &Path, base: &str) -> Result<Vec<String>, Error> {
    let range = format!("{base}...HEAD");
    let mut files = BTreeSet::new();
    for args in [
        &["diff", "--name-only", range.as_str()][..],
        &["diff", "--name-only", "HEAD"],
        &["ls-files", "--others", "--exclude-standard"],
    ] {
        files.extend(git_lines(root, args)?);
    }
    Ok(files.into_iter().collect())
}

/// Runs `jobs` one after another in [`WORKSPACE`], each with its output shown; every job runs,
/// and the names of those that failed are returned.
///
/// # Errors
///
/// [`Error::Tool`] when a job can't start.
pub fn run(root: &Path, jobs: &[Job]) -> Result<Vec<String>, Error> {
    let mut failed = Vec::new();
    for j in jobs {
        let Some((program, args)) = j.argv.split_first() else {
            continue;
        };
        eprintln!("cobot: scope: {}", j.name);
        let status = std::process::Command::new(program)
            .args(args)
            .current_dir(root.join(WORKSPACE))
            .status()
            .map_err(|e| Error::Tool {
                command: program.clone(),
                message: e.to_string(),
            })?;
        if !status.success() {
            failed.push(j.name.clone());
        }
    }
    Ok(failed)
}

/// What an `architecture` site draws from, besides its own unit: everything
/// `architecture.describe` reads.
const DESCRIBED: [&str; 5] = [
    "config.yaml",
    "infra/",
    "catalog/",
    "workloads/",
    ".config/",
];

fn relative(root: &Path, p: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    p.strip_prefix(&root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The instance's manifest sites a change to `main` touching `files` must publish, by unit; `files` of `None`
/// means every site, as on a first deploy. A site's inputs are its unit (the instance's
/// `config.yaml` for a manifest unit, else its directory), its content, and, on the
/// `architecture` template, everything that template draws.
///
/// # Errors
///
/// As [`crate::sites::find_in`].
pub fn deploys(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    // `workloads/` is the legacy layout, published from its own instance's account.
    let sites: Vec<_> = crate::sites::find_in(root, root, &[])?
        .into_iter()
        .filter(|s| !s.unit.starts_with("workloads/"))
        .collect();
    let Some(files) = files else {
        return Ok(sites.into_iter().map(|s| s.unit).collect());
    };
    Ok(sites
        .into_iter()
        .filter(|s| {
            let dir = relative(root, &s.dir);
            let unit = if dir.is_empty() {
                "config.yaml".to_string()
            } else {
                format!("{dir}/")
            };
            let content = format!("{}/", relative(root, &s.content));
            let mut inputs = vec![unit, content];
            if s.template == "architecture" {
                inputs.extend(DESCRIBED.iter().map(|p| (*p).to_string()));
            }
            files.iter().any(|f| {
                inputs.iter().any(|i| {
                    i != "/" && (f == i || (i.ends_with('/') && f.starts_with(i.as_str())))
                })
            })
        })
        .map(|s| s.unit)
        .collect())
}

/// Paths whose change makes the job image too old for the Worker: the image's tools and its
/// entrypoint. A Worker
/// deploy waits for a new image; jobs run `main`'s newest `pigeon` from `builds/`, so its code
/// is not among them; see `.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md`
/// §3.
pub const IMAGE_BLOCKS: [&str; 4] = [
    "src/pigeon-cli/Dockerfile",
    "src/pigeon-cli/pigeon-start",
    "mise.toml",
    "src/pigeon-cli/rust-toolchain.toml",
];

/// The files among `files` that make the job image block a Worker deploy.
#[must_use]
pub fn image_inputs(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| IMAGE_BLOCKS.contains(&f.as_str()))
        .cloned()
        .collect()
}

/// Whether a deploy of `main` touching `files` builds `pigeon` again: when any of the Rust
/// workspace changed; `files` of `None` means everything.
#[must_use]
pub fn builds(files: Option<&[String]>) -> bool {
    files.is_none_or(|f| f.iter().any(|f| f.starts_with("src/pigeon-cli/")))
}

/// The Workers a change to `main` touching `files` must deploy, by name; `files` of `None`
/// means every Worker. A Worker's inputs are its spec's directory, its migrations, its
/// Dockerfile, the workspace's build settings, and the crates its entry crate builds from.
///
/// # Errors
///
/// As [`crate::workers::discover`], or a workspace manifest that can't be read.
pub fn workers(root: &Path, files: Option<&[String]>) -> Result<Vec<String>, Error> {
    let found = crate::workers::discover(root)?;
    let Some(files) = files else {
        return Ok(found
            .iter()
            .map(|f| crate::workers::deployed_name(root, f))
            .collect());
    };
    let crates = crates(root)?;
    let mut out = Vec::new();
    for f in found {
        let mut inputs: Vec<String> = vec![format!("{}/", f.dir.trim_end_matches('/'))];
        inputs.extend(
            f.spec
                .bindings
                .d1
                .values()
                .filter_map(|d| d.migrations.as_ref())
                .map(|m| format!("{}/", m.trim_end_matches('/'))),
        );
        inputs.extend(f.spec.containers.iter().map(|c| c.image.dockerfile.clone()));
        inputs.extend(EVERYTHING.iter().map(|p| (*p).to_string()));
        let entry = format!("{}/", f.spec.entry.krate.trim_end_matches('/'));
        if let Some(package) = crates.by_dir.get(&entry) {
            let mut wanted = BTreeSet::from([package.clone()]);
            loop {
                let more: Vec<String> = wanted
                    .iter()
                    .flat_map(|p| crates.deps.get(p).into_iter().flatten())
                    .filter(|d| !wanted.contains(*d))
                    .cloned()
                    .collect();
                if more.is_empty() {
                    break;
                }
                wanted.extend(more);
            }
            inputs.extend(
                crates
                    .by_dir
                    .iter()
                    .filter(|(_, p)| wanted.contains(*p))
                    .map(|(d, _)| d.clone()),
            );
        } else {
            inputs.push(entry);
        }
        let hit = files.iter().any(|file| {
            inputs
                .iter()
                .any(|i| file == i || (i.ends_with('/') && file.starts_with(i.as_str())))
        });
        if hit {
            out.push(crate::workers::deployed_name(root, &f));
        }
    }
    Ok(out)
}
