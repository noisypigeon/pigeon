//! `main`'s test map, `tests.json`, and the tests it picks for a change; see the 2026-11-11
//! amendment to `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`
//! and `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §3.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Crates, Job};

/// One unit of the test map: a test binary, or one top-level module of a `suite` binary, and
/// the workspace files it ran.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Binary {
    /// Its package.
    pub package: String,
    /// `lib`, `bin` or `test`.
    pub kind: String,
    /// Its target's name.
    pub name: String,
    /// `""` for the whole target, else `<module>::`, the libtest filter of one module.
    #[serde(default)]
    pub filter: String,
    /// The repository paths it ran.
    #[serde(default)]
    pub files: Vec<String>,
}

/// `main`'s test map, `tests.json`, as `super::TEST_MAP` writes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestMap {
    /// The commit it maps.
    #[serde(default)]
    pub sha: String,
    /// Its units.
    #[serde(default)]
    pub binaries: Vec<Binary>,
}

/// The integration target holding a crate's modules, `tests/suite/main.rs`.
const SUITE: &str = "suite";

impl TestMap {
    /// The test checks for `files`, from the units that ran them; none when the map can't say:
    /// a non-`.rs` file under a crate, a `.rs` file it doesn't know, or more than half of its
    /// units. A wasm-only file picks none, and an unknown unit-test module its package's `lib`;
    /// see `.claude/decisions/2026-11-15-development-code-checks-by-reach.md`. A package's whole
    /// targets share one check, and its suite's modules another.
    pub(super) fn select(&self, crates: &Crates, files: &[&str]) -> Option<Vec<Job>> {
        let mut picked: BTreeSet<&Binary> = BTreeSet::new();
        for f in files.iter().filter(|f| !crates.layout.wasm_only(f)) {
            let Some((dir, package)) = crates
                .by_dir
                .iter()
                .find(|(d, _)| f.starts_with(d.as_str()))
            else {
                continue;
            };
            let rel = &f[dir.len()..];
            if Path::new(rel).extension().is_none_or(|e| e != "rs") {
                return None;
            }
            let own = Own::of(rel);
            let mut reached: Vec<&Binary> = self
                .binaries
                .iter()
                .filter(|b| own.picks(package, b) || b.files.iter().any(|x| x == f))
                .collect();
            if reached.is_empty() && unit_tests(rel) {
                reached = self
                    .binaries
                    .iter()
                    .filter(|b| b.package == *package && b.kind == "lib")
                    .collect();
            }
            if reached.is_empty() {
                return None;
            }
            picked.extend(reached);
        }
        if picked.len() * 2 > self.binaries.len() {
            return None;
        }
        let picked: BTreeSet<Binary> = picked
            .into_iter()
            .map(|b| current(crates, b))
            .collect::<Option<_>>()?;
        let mut whole: BTreeMap<&str, BTreeSet<&Binary>> = BTreeMap::new();
        let mut modules: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for b in &picked {
            if b.filter.is_empty() {
                whole.entry(&b.package).or_default().insert(b);
            }
        }
        for b in &picked {
            let suite_whole = whole
                .get(b.package.as_str())
                .is_some_and(|w| w.iter().any(|x| x.kind == "test" && x.name == b.name));
            if !b.filter.is_empty() && !suite_whole {
                modules.entry(&b.package).or_default().insert(&b.filter);
            }
        }
        let packages: BTreeSet<&str> = whole.keys().chain(modules.keys()).copied().collect();
        let mut jobs = Vec::new();
        for package in packages {
            if let Some(bins) = whole.get(package) {
                jobs.push(whole_job(package, bins));
            }
            if let Some(filters) = modules.get(package) {
                jobs.push(suite_job(package, filters));
            }
        }
        Some(jobs)
    }
}

/// One part of the tests a check runs: a package's whole tests when `kind` is empty, else one
/// target, or one module of it when `filter` is set; filters are opaque.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Unit {
    /// Its package.
    pub package: String,
    /// `lib`, `bin` or `test`; empty for the whole package.
    pub kind: String,
    /// Its target's name.
    pub name: String,
    /// `""` for the whole target, else the libtest filter of one module.
    pub filter: String,
}

impl Unit {
    /// `package`'s whole tests.
    #[must_use]
    pub fn whole(package: &str) -> Self {
        Self {
            package: package.into(),
            kind: String::new(),
            name: String::new(),
            filter: String::new(),
        }
    }

    /// Target `name` of `package`, of `kind`, or its module `filter`.
    #[must_use]
    pub fn target(package: &str, kind: &str, name: &str, filter: &str) -> Self {
        Self {
            package: package.into(),
            kind: kind.into(),
            name: name.into(),
            filter: filter.into(),
        }
    }
}

/// `b` as the workspace now runs it: none when its package or binary is gone, as after a rename; a whole
/// test target that is gone runs as the module of its name in its package's `suite`, and none
/// when there is no suite; see
/// `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §3.
fn current(crates: &Crates, b: &Binary) -> Option<Binary> {
    if !crates.by_dir.values().any(|p| *p == b.package) {
        return None;
    }
    if b.kind == "bin"
        && crates
            .bins
            .get(&b.package)
            .is_some_and(|bins| !bins.contains(&b.name))
    {
        return None;
    }
    let Some(targets) = crates.tests.get(&b.package) else {
        return Some(b.clone());
    };
    if b.kind != "test" || !b.filter.is_empty() || targets.contains(&b.name) {
        return Some(b.clone());
    }
    targets.contains(SUITE).then(|| Binary {
        name: SUITE.into(),
        filter: format!("{}::", b.name),
        ..b.clone()
    })
}

/// What a changed path picks by itself, before coverage: `tests/<x>.rs` its own binary,
/// `tests/suite/<m>.rs` or `tests/suite/<m>/**` its module, and the suite's `main.rs` or
/// `harness/**` the whole suite.
enum Own<'a> {
    None,
    Binary(&'a str),
    Module(&'a str),
    Suite,
}

impl<'a> Own<'a> {
    fn of(rel: &'a str) -> Self {
        let Some(t) = rel.strip_prefix("tests/") else {
            return Self::None;
        };
        if let Some(s) = t.strip_prefix("suite/") {
            if s == "main.rs" || s.starts_with("harness/") || s == "harness.rs" {
                return Self::Suite;
            }
            let m = s.split('/').next().unwrap_or(s);
            return Self::Module(m.strip_suffix(".rs").unwrap_or(m));
        }
        match t.strip_suffix(".rs") {
            Some(n) if !n.contains('/') => Self::Binary(n),
            _ => Self::None,
        }
    }

    fn picks(&self, package: &str, b: &Binary) -> bool {
        let test = b.package == package && b.kind == "test";
        match self {
            Self::None => false,
            Self::Binary(n) => test && b.name == *n && b.filter.is_empty(),
            Self::Module(m) => test && b.name == SUITE && b.filter.strip_suffix("::") == Some(*m),
            Self::Suite => test && b.name == SUITE,
        }
    }
}

/// One check running `bins`, whole targets of `package`, as today.
fn whole_job(package: &str, bins: &BTreeSet<&Binary>) -> Job {
    let mut argv = vec![
        "cargo".to_string(),
        "test".into(),
        "-p".into(),
        package.into(),
    ];
    let mut names = Vec::new();
    for b in bins {
        if b.kind == "lib" {
            argv.push("--lib".into());
            names.push("lib".to_string());
        } else {
            argv.extend([format!("--{}", b.kind), b.name.clone()]);
            names.push(b.name.clone());
        }
    }
    argv.push("--locked".into());
    Job {
        name: format!("test {package}: {}", names.join(" ")),
        argv,
        units: bins
            .iter()
            .map(|b| Unit::target(&b.package, &b.kind, &b.name, ""))
            .collect(),
    }
}

/// One check running `filters`, modules of `package`'s suite; libtest ORs them.
fn suite_job(package: &str, filters: &BTreeSet<&str>) -> Job {
    let mut argv = vec![
        "cargo".to_string(),
        "test".into(),
        "-p".into(),
        package.into(),
        "--test".into(),
        SUITE.into(),
        "--locked".into(),
        "--".into(),
    ];
    argv.extend(filters.iter().map(|f| (*f).to_string()));
    let names: Vec<&str> = filters.iter().map(|f| f.trim_end_matches("::")).collect();
    Job {
        name: format!("test {package}: {SUITE} {}", names.join(" ")),
        argv,
        units: filters
            .iter()
            .map(|f| Unit::target(package, "test", SUITE, f))
            .collect(),
    }
}

/// Whether `rel`, a path in its crate, is a unit-test module of its `lib`: `src/**/tests.rs` or
/// `src/**/tests/*.rs`.
fn unit_tests(rel: &str) -> bool {
    rel.strip_prefix("src/").is_some_and(|r| {
        r == "tests.rs"
            || r.ends_with("/tests.rs")
            || r.starts_with("tests/")
            || r.contains("/tests/")
    })
}
