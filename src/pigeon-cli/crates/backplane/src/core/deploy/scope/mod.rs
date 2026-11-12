//! The checks a change needs, from the paths it touches, and what a deploy of `main` builds; the
//! selection only, from inputs the shell reads; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §4.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::cobot::release::Config;

mod map;

use super::gate::{Gate, MAX_PACKAGES};

pub use map::{Binary, TestMap};

/// The Rust workspace, from the root.
pub const WORKSPACE: &str = "src/pigeon-cli";

/// The release configuration, from the root.
pub const RELEASES: &str = ".config/releases.yaml";

/// Paths whose change runs every check.
pub const EVERYTHING: [&str; 5] = [
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

/// The iOS app, which its `ios` check builds on macOS; see
/// `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`.
const IOS: [&str; 1] = ["src/pigeon-ios/"];

/// The crates whose `worker` build compiles to wasm.
const WORKER: [&str; 1] = ["pigeon-backplane"];

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

/// The workspace's crates, as the shell reads them from its manifests: each crate directory's package, and each package's workspace
/// dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Crates {
    /// Each crate directory's package.
    pub by_dir: BTreeMap<String, String>,
    /// Each package's workspace dependencies.
    pub deps: BTreeMap<String, BTreeSet<String>>,
    /// `pigeon`'s features, each with the packages it enables.
    pub features: BTreeMap<String, BTreeSet<String>>,
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
    out.extend(worker_jobs(&crates.of(&WORKER)));
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
    if crates.contains(&"pigeon-backplane") {
        out.push(clippy(
            "clippy backplane worker (wasm)",
            "pigeon-backplane",
            true,
        ));
    }
    out
}

/// The Rust checks for the crates `files` touch and their dependants; with `map`, their tests are
/// the binaries [`TestMap::select`] picks.
fn rust_jobs(crates: &Crates, config: &Config, files: &[&str], map: Option<&TestMap>) -> Vec<Job> {
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
        match map.and_then(|m| m.select(crates, files)) {
            Some(picked) => out.extend(picked),
            None => out.push(with_p("test", &["--locked"])),
        }
    }
    out
}

/// How much of the full set the merge gate of a change touching `files` runs: scoped when the
/// test map chose its tests, they span at most [`MAX_PACKAGES`] packages, and no workspace-wide
/// path changed; full otherwise, saying why. See the 2026-11-12 amendment on the merge gate to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
#[must_use]
pub fn gate_of(crates: &Crates, files: &[String], map: Option<&TestMap>) -> Gate {
    let files: Vec<&str> = files
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .collect();
    if let Some(f) = files.iter().find(|f| under(f, &EVERYTHING)) {
        return Gate::Full(format!("{f} runs every check"));
    }
    let rust = files
        .iter()
        .any(|f| crates.by_dir.keys().any(|d| f.starts_with(d.as_str())));
    if !rust {
        return Gate::Scoped;
    }
    let Some(map) = map else {
        return Gate::Full("there is no test map for main's newest build".into());
    };
    let Some(picked) = map.select(crates, &files) else {
        return Gate::Full("the test map can't choose this change's tests".into());
    };
    if picked.len() > MAX_PACKAGES {
        return Gate::Full(format!("its tests span {} packages", picked.len()));
    }
    Gate::Scoped
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
    let mut out = Vec::new();
    if files.iter().any(|f| under(f, &EVERYTHING)) {
        out.extend(everything(&all_features, crates));
    } else {
        out.extend(rust_jobs(crates, config, &files, map));
    }
    let ran_backplane = ran_tests_of(&out, "pigeon-backplane");
    if files.iter().any(|f| under(f, &INFRA)) {
        out.push(Job {
            name: "infra.validate".into(),
            argv: pigeon(&["backplane", "run", "infra.validate", "--root", "../.."]),
        });
        if !ran_backplane {
            out.push(job(
                "test pigeon-backplane: cobot_catalog cobot_manifest cobot_apex cobot_git cobot_credentials",
                &[
                    "cargo",
                    "test",
                    "-p",
                    "pigeon-backplane",
                    "--locked",
                    "--test",
                    "cobot_catalog",
                    "--test",
                    "cobot_manifest",
                    "--test",
                    "cobot_apex",
                    "--test",
                    "cobot_git",
                    "--test",
                    "cobot_credentials",
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
    if files.iter().any(|f| under(f, &IOS)) {
        out.push(job("ios", &["bash", crate::actions::IOS_CHECK]));
    }
    out.extend(spikes(&files));
    if files.contains(&TEST_MAP) {
        out.push(test_map());
    }
    out.push(Job {
        name: "policy.check".into(),
        argv: pigeon(&["backplane", "run", "policy.check", "--root", "../.."]),
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
    }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Crates, WORKER, everything};

    #[test]
    fn a_workspace_setting_builds_only_the_workers_the_tree_has() {
        let crates = Crates {
            by_dir: [(
                "src/pigeon-cli/crates/backplane/".to_string(),
                "pigeon-backplane".to_string(),
            )]
            .into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
        };
        assert_eq!(crates.of(&WORKER), ["pigeon-backplane"]);
        let names: Vec<String> = everything(&BTreeSet::default(), &crates)
            .into_iter()
            .map(|j| j.name)
            .collect();
        assert!(
            names.contains(&"clippy backplane worker (wasm)".to_string()),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("cobot")), "{names:?}");
    }

    #[test]
    fn a_gate_is_scoped_only_when_the_test_map_chose_a_narrow_set() {
        use super::{Binary, Gate, TestMap, gate_of};
        let dir = |p: &str| format!("src/pigeon-cli/crates/{p}/");
        let crates = Crates {
            by_dir: [
                (dir("backplane"), "pigeon-backplane".to_string()),
                (dir("pigeon"), "pigeon".to_string()),
                (dir("sentinel"), "pigeon-sentinel".to_string()),
            ]
            .into(),
            deps: BTreeMap::default(),
            features: BTreeMap::default(),
        };
        let bin = |package: &str, name: &str, file: &str| Binary {
            package: package.into(),
            kind: "test".into(),
            name: name.into(),
            files: vec![file.into()],
        };
        let a = "src/pigeon-cli/crates/backplane/src/a.rs";
        let b = "src/pigeon-cli/crates/pigeon/src/b.rs";
        let c = "src/pigeon-cli/crates/sentinel/src/c.rs";
        let map = TestMap {
            sha: "x".into(),
            binaries: vec![
                bin("pigeon-backplane", "a", a),
                bin("pigeon", "b", b),
                bin("pigeon-sentinel", "c", c),
                bin("pigeon-backplane", "d", "elsewhere.rs"),
                bin("pigeon", "e", "elsewhere.rs"),
                bin("pigeon-sentinel", "f", "elsewhere.rs"),
                bin("pigeon-backplane", "g", "elsewhere.rs"),
            ],
        };
        let files = |f: &[&str]| f.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let scoped = |g: &Gate| *g == Gate::Scoped;
        assert!(scoped(&gate_of(&crates, &files(&[a]), Some(&map))));
        assert!(
            scoped(&gate_of(&crates, &files(&[a, b]), Some(&map))),
            "2 packages"
        );
        assert!(
            scoped(&gate_of(&crates, &files(&["docs/x.md"]), None)),
            "no Rust"
        );
        for (name, f, m) in [
            ("3 packages", files(&[a, b, c]), Some(&map)),
            ("no map", files(&[a]), None),
            (
                "unknown to the map",
                files(&["src/pigeon-cli/crates/pigeon/src/new.rs"]),
                Some(&map),
            ),
            ("workspace-wide", files(&[a, "mise.toml"]), Some(&map)),
        ] {
            assert!(!scoped(&gate_of(&crates, &f, m)), "{name}");
        }
    }
}
