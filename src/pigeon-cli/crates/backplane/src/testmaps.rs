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
}

impl TestMaps {
    /// `main`'s commit `sha` reached GitHub: its map is built, and an older commit's is dropped.
    pub fn mirrored(&mut self, sha: &str) {
        if self.latest_map.as_ref().is_some_and(|m| m.sha == sha)
            || self.actions.dispatches.iter().any(|d| d.sha == sha)
        {
            return;
        }
        self.actions.keep_head(Some(sha));
        self.actions.queue(MAIN, sha, CHECK);
        self.copying = None;
        self.failed = None;
    }

    /// Whether anything waits on GitHub or a copy.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.copying.is_some() || !self.actions.dispatches.is_empty()
    }

    /// Entry `n`'s check finished as `f`: a passed one is copied from its run.
    pub fn finished(&mut self, n: usize, f: &Finished) {
        let Some(d) = self.actions.dispatches.get(n) else {
            return;
        };
        if f.ok {
            self.copying = d.run.map(|run| (d.sha.clone(), run));
        } else {
            self.failed = Some(format!("the test map of {} failed", d.sha));
        }
    }

    /// Drops the dispatches whose check has finished.
    pub fn prune(&mut self) {
        self.actions
            .dispatches
            .retain(|d| !d.checks.iter().all(|c| d.reported.contains(c)));
    }

    /// The map of `sha` was copied, with `sha256`.
    pub fn copied(&mut self, sha: &str, sha256: &str) {
        self.copying = None;
        self.latest_map = Some(Map {
            sha: sha.into(),
            sha256: sha256.into(),
        });
    }

    /// The copy of `sha`'s map failed, for `why`; it waits for the next commit.
    pub fn copy_failed(&mut self, sha: &str, why: &str) {
        self.copying = None;
        self.failed = Some(format!("the test map of {sha} was not copied: {why}"));
    }

    /// What `MainRun`'s status shows.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "dispatches": self.actions.view(),
            "copying": self.copying.as_ref().map(|(sha, _)| sha),
            "latest_map": self.latest_map,
            "failed": self.failed,
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
