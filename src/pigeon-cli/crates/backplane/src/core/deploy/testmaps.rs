//! The test map of `main`: `MainRun` dispatches the `test map` check on each commit that reaches
//! GitHub, as change 0, and keeps the newest map beside the build; see the 2026-11-11 amendment to
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::{self, Actions, Finished};
use crate::changes::Check;

/// The check's name, as `crate::cobot::scope::test_map` names it.
pub const CHECK: &str = "test map";

/// The run's artifact: the map, unzipped.
pub const ARTIFACT: &str = "tests.json";

/// The change number `main`'s dispatches carry.
pub const MAIN: u32 = 0;

/// Where the map of `sha` is kept, beside `builds/<sha>/pigeon`.
#[must_use]
pub fn key(sha: &str) -> String {
    format!("builds/{sha}/{ARTIFACT}")
}

/// The check, as `ci.yml` runs it.
#[must_use]
pub fn check() -> Check {
    Check {
        name: CHECK.into(),
        argv: vec!["bash".into(), actions::TEST_MAP.into()],
    }
}

/// A stored map: its commit and its SHA-256, hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    /// The commit of `main` it maps.
    pub sha: String,
    /// The file's SHA-256.
    pub sha256: String,
}

/// `MainRun`'s test maps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestMaps {
    /// The dispatch of the newest commit's check.
    #[serde(default)]
    pub actions: Actions,
    /// The commit and run whose map is to be copied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copying: Option<(String, u64)>,
    /// The newest stored map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_map: Option<Map>,
    /// Why the newest commit has no map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
    /// A mirrored commit waiting to be compared with the newest map's commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deciding: Option<String>,
    /// The newest commit that changed no Rust input, so the newest map stands for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// A failed map's commit, until a task is opened for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_sha: Option<String>,
}

/// Whether `files` hold a Rust input, whose change can change what the tests run.
#[must_use]
pub fn rust_inputs(files: &[String]) -> bool {
    files.iter().any(|f| {
        let name = f.rsplit('/').next().unwrap_or(f);
        std::path::Path::new(f)
            .extension()
            .is_some_and(|e| e == "rs")
            || matches!(name, "Cargo.toml" | "Cargo.lock")
            || f.contains("/.cargo/")
            || f == "mise.toml"
            || f == ".config/releases.yaml"
    })
}

/// The most files GitHub's compare lists; a compare that reaches it may hold more.
pub const COMPARE_FILES: usize = 300;

/// Whether GitHub's compare `v` holds a Rust input: a listing cut at [`COMPARE_FILES`], or one
/// that can't be read, counts as one.
#[must_use]
pub fn rust_since(v: &Value) -> bool {
    let Some(files) = v["files"].as_array() else {
        return true;
    };
    let names: Vec<String> = files
        .iter()
        .filter_map(|f| f["filename"].as_str().map(str::to_string))
        .collect();
    files.len() >= COMPARE_FILES || rust_inputs(&names)
}

/// The change a squash on `main` merged, from its subject's `(!n)`.
#[must_use]
pub fn merged_change(message: &str) -> Option<u32> {
    let subject = message.lines().next()?.trim_end();
    let open = subject.rfind("(!")?;
    subject[open + 2..].strip_suffix(')')?.parse().ok()
}

impl TestMaps {
    /// `main`'s commit `sha` reached GitHub: the next maps, with its map built and an older
    /// commit's dropped.
    ///
    /// While a map exists, the commit waits in `deciding` to be compared with it; see the factory
    /// ADR's 2026-11-12 amendment.
    #[must_use]
    pub fn mirrored(&self, sha: &str) -> Self {
        if self.latest_map.as_ref().is_some_and(|m| m.sha == sha)
            || self.actions.dispatches.iter().any(|d| d.sha == sha)
            || self.deciding.as_deref() == Some(sha)
        {
            return self.clone();
        }
        if self.latest_map.is_some() {
            let mut next = self.clone();
            next.actions.keep_head(None);
            next.deciding = Some(sha.into());
            return next;
        }
        self.map(sha)
    }

    /// The commit in `deciding`, `sha`, compared: mapped when it changed a Rust input, or else
    /// recorded as skipped.
    #[must_use]
    pub fn decided(&self, sha: &str, rust: bool) -> Self {
        if self.deciding.as_deref() != Some(sha) {
            return self.clone();
        }
        let mut next = self.clone();
        next.deciding = None;
        if rust {
            return next.map(sha);
        }
        next.skipped = Some(sha.into());
        next
    }

    /// It, with `sha`'s map queued and an older commit's dropped.
    fn map(&self, sha: &str) -> Self {
        let mut next = self.clone();
        next.actions.keep_head(Some(sha));
        next.actions.queue(MAIN, sha, CHECK);
        next.copying = None;
        next.failed = None;
        next
    }

    /// Whether anything waits on GitHub or a copy.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.copying.is_some() || !self.actions.dispatches.is_empty() || self.deciding.is_some()
    }

    /// Entry `n`'s check finished as `f`: a passed one is copied from its run.
    #[must_use]
    pub fn finished(&self, n: usize, f: &Finished) -> Self {
        let mut next = self.clone();
        let Some(d) = self.actions.dispatches.get(n) else {
            return next;
        };
        if f.ok {
            next.copying = d.run.map(|run| (d.sha.clone(), run));
        } else {
            next.failed = Some(format!("the test map of {} failed", d.sha));
            next.failed_sha = Some(d.sha.clone());
        }
        next
    }

    /// It without the dispatches whose check has finished.
    #[must_use]
    pub fn prune(&self) -> Self {
        let mut next = self.clone();
        next.actions
            .dispatches
            .retain(|d| !d.checks.iter().all(|c| d.reported.contains(c)));
        next
    }

    /// The map of `sha` was copied, with `sha256`.
    #[must_use]
    pub fn copied(&self, sha: &str, sha256: &str) -> Self {
        let mut next = self.clone();
        next.copying = None;
        next.latest_map = Some(Map {
            sha: sha.into(),
            sha256: sha256.into(),
        });
        next
    }

    /// The copy of `sha`'s map failed, for `why`; it waits for the next commit.
    #[must_use]
    pub fn copy_failed(&self, sha: &str, why: &str) -> Self {
        let mut next = self.clone();
        next.copying = None;
        next.failed = Some(format!("the test map of {sha} was not copied: {why}"));
        next
    }

    /// What `MainRun`'s status shows.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "dispatches": self.actions.view(),
            "copying": self.copying.as_ref().map(|(sha, _)| sha),
            "latest_map": self.latest_map,
            "failed": self.failed,
            "deciding": self.deciding,
            "skipped": self.skipped,
        })
    }
}

/// Why `body` is not the map of `sha`; none when it is.
#[must_use]
pub fn invalid(body: &[u8], sha: &str) -> Option<String> {
    if body.starts_with(b"PK\x03\x04") {
        return Some("the artifact is zipped".into());
    }
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return Some(format!("the artifact is not JSON: {e}")),
    };
    if v["sha"].as_str() != Some(sha) {
        return Some(format!("the artifact maps {}, not {sha}", v["sha"]));
    }
    if !v["binaries"].is_array() {
        return Some("the artifact lists no binaries".into());
    }
    None
}
