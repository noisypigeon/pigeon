//! `main`'s test map, `tests.json`, and the tests it picks for a change; see the 2026-11-11
//! amendment to `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Crates, Job};

/// One test binary in the test map, and the workspace files it ran.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Binary {
    /// Its package.
    pub package: String,
    /// `lib`, `bin` or `test`.
    pub kind: String,
    /// Its target's name.
    pub name: String,
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
    /// Its binaries.
    #[serde(default)]
    pub binaries: Vec<Binary>,
}

impl TestMap {
    /// The test checks for `files`, one per package, from the binaries that ran them; none when
    /// the map can't say: a non-`.rs` file under a crate, a `.rs` file it doesn't know, or more
    /// than half of its binaries.
    pub(super) fn select(&self, crates: &Crates, files: &[&str]) -> Option<Vec<Job>> {
        let mut picked: BTreeSet<&Binary> = BTreeSet::new();
        for f in files {
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
            let own = rel
                .strip_prefix("tests/")
                .and_then(|t| t.strip_suffix(".rs"))
                .filter(|t| !t.contains('/'));
            let reached: Vec<&Binary> = self
                .binaries
                .iter()
                .filter(|b| {
                    own.is_some_and(|t| b.package == *package && b.kind == "test" && b.name == t)
                        || b.files.iter().any(|x| x == f)
                })
                .collect();
            if reached.is_empty() {
                return None;
            }
            picked.extend(reached);
        }
        if picked.len() * 2 > self.binaries.len() {
            return None;
        }
        let mut by_package: BTreeMap<&str, Vec<&Binary>> = BTreeMap::new();
        for b in picked {
            by_package.entry(&b.package).or_default().push(b);
        }
        Some(
            by_package
                .into_iter()
                .map(|(package, bins)| {
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
                        } else {
                            argv.extend([format!("--{}", b.kind), b.name.clone()]);
                        }
                        names.push(if b.kind == "lib" {
                            "lib".to_string()
                        } else {
                            b.name.clone()
                        });
                    }
                    argv.push("--locked".into());
                    Job {
                        name: format!("test {package}: {}", names.join(" ")),
                        argv,
                    }
                })
                .collect(),
        )
    }
}
