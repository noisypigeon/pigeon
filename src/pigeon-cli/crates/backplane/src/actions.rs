//! A change's code checks on GitHub Actions: `ChangeRun`'s dispatches of `main`'s `ci.yml` as
//! pure transitions, which the Worker carries out; see
//! `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §3–§5.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::changes::Check;

/// The workflow dispatched, always from `main`.
pub const WORKFLOW: &str = "ci.yml";

/// The branch whose workflow runs.
pub const REF: &str = "main";

/// How often a run is polled, in seconds.
pub const POLL: u64 = 30;

/// How long a dispatched run may take to appear before it is dispatched again, in seconds.
pub const FIND_WITHIN: u64 = 600;

/// The waits after the first, second and third failure, and every one after.
const BACKOFF: [u64; 3] = [60, 300, 1800];

/// The cargo subcommands that are code checks.
const CODE: [&str; 5] = ["fmt", "clippy", "test", "build", "check"];

/// The test map's script, as `crate::cobot::scope::test_map` runs it from the workspace.
pub const TEST_MAP: &str = "../../tooling/ci/test-map.sh";

/// The `ios` check's script, as `crate::cobot::scope` runs it; it needs macOS, which only GitHub
/// gives; see `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`.
pub const IOS_CHECK: &str = "../pigeon-ios/check.sh";

/// Whether a check with `argv` is a code check, run on GitHub: `cargo fmt|clippy|test|build|check`,
/// the test map, or the `ios` check.
#[must_use]
pub fn on_github(argv: &[String]) -> bool {
    match argv {
        [p, v, ..] if p == "cargo" => CODE.contains(&v.as_str()),
        [p, script] => p == "bash" && (script == TEST_MAP || script == IOS_CHECK),
        _ => false,
    }
}

/// One dispatch of [`WORKFLOW`] on a head.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch {
    /// The head it checks.
    pub sha: String,
    /// The control plane's ID for it, which the run's title carries.
    pub id: String,
    /// The checks it runs.
    pub checks: Vec<String>,
    /// When it was dispatched; none until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<u64>,
    /// The run's ID, once found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<u64>,
    /// The run's page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The checks whose results are applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reported: Vec<String>,
    /// Failures since the last success.
    #[serde(default)]
    pub failures: u32,
    /// Nothing is due before this, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_at: Option<u64>,
    /// Why its checks wait on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<String>,
}

impl Dispatch {
    fn done(&self) -> bool {
        self.checks.iter().all(|c| self.reported.contains(c))
    }

    fn due(&self, now: u64) -> bool {
        !self.done() && self.next_at.is_none_or(|t| now >= t)
    }

    fn fail(&mut self, why: String, now: u64) {
        let i = usize::try_from(self.failures)
            .unwrap_or(usize::MAX)
            .min(BACKOFF.len() - 1);
        self.failures += 1;
        self.next_at = Some(now + BACKOFF[i]);
        self.waiting = Some(why);
    }
}

/// A change's dispatches, as `ChangeRun` keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actions {
    /// Its dispatches, oldest first.
    #[serde(default)]
    pub dispatches: Vec<Dispatch>,
    /// The last dispatch number used.
    #[serde(default)]
    pub seq: u32,
    /// The head the owner moved to Cloudflare's `Checker`; its code checks never dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloudflare: Option<String>,
}

/// Where the change's `mirror` job is on the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    /// The head is on GitHub.
    Passed,
    /// It is being pushed.
    Open,
    /// It failed, or there is none.
    Failed,
}

/// What the Worker must do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    /// Run the `mirror` job again.
    Mirror,
    /// Dispatch entry `n`.
    Dispatch(usize),
    /// Look for entry `n`'s run.
    Find(usize),
    /// Poll entry `n`'s run.
    Poll(usize),
}

/// A check's result on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// The check.
    pub name: String,
    /// Whether it passed.
    pub ok: bool,
    /// Its GitHub job, whose log is copied; none when the run never ran it.
    pub job: Option<u64>,
}

/// A run's title for dispatch `id`, as `ci.yml`'s `run-name` writes it, ends with this.
fn title_suffix(id: &str) -> String {
    format!("({id})")
}

impl Actions {
    /// Queues `check` on head `sha` of change `number`, into the head's undispatched entry.
    pub fn queue(&mut self, number: u32, sha: &str, check: &str) {
        if let Some(d) = self
            .dispatches
            .iter_mut()
            .find(|d| d.sha == sha && d.dispatched_at.is_none())
        {
            if !d.checks.iter().any(|c| c == check) {
                d.checks.push(check.to_string());
            }
            return;
        }
        self.seq += 1;
        self.dispatches.push(Dispatch {
            sha: sha.to_string(),
            id: format!("{number}-{}-{}", &sha[..12.min(sha.len())], self.seq),
            checks: vec![check.to_string()],
            ..Dispatch::default()
        });
    }

    /// Drops every entry not on `head`, so an old head's results are never applied; a new head
    /// goes back to GitHub.
    pub fn keep_head(&mut self, head: Option<&str>) {
        self.dispatches.retain(|d| Some(d.sha.as_str()) == head);
        if self.cloudflare.as_deref() != head {
            self.cloudflare = None;
        }
    }

    /// Whether the owner moved head `sha`'s code checks to Cloudflare.
    #[must_use]
    pub fn on_cloudflare(&self, sha: &str) -> bool {
        self.cloudflare.as_deref() == Some(sha)
    }

    /// Keeps only the checks still `open` on the change: an undispatched entry loses the others
    /// and goes when empty, and a dispatched one stops following them.
    pub fn keep_open(&mut self, open: &[String]) {
        for d in &mut self.dispatches {
            if d.dispatched_at.is_none() {
                d.checks.retain(|c| open.contains(c));
            } else {
                let closed: Vec<String> = d
                    .checks
                    .iter()
                    .filter(|c| !open.contains(c) && !d.reported.contains(c))
                    .cloned()
                    .collect();
                d.reported.extend(closed);
            }
        }
        self.dispatches
            .retain(|d| d.dispatched_at.is_some() || !d.checks.is_empty());
    }

    /// Whether `check` on `sha` waits on GitHub, not a Container.
    #[must_use]
    pub fn owns(&self, sha: &str, check: &str) -> bool {
        self.dispatches.iter().any(|d| {
            d.sha == sha
                && d.checks.iter().any(|c| c == check)
                && !d.reported.iter().any(|c| c == check)
        })
    }

    /// What is due at `now`, with the head's `mirror` job where it is.
    pub fn due(&mut self, mirror: Mirror, now: u64) -> Vec<Due> {
        let mut out = Vec::new();
        for (i, d) in self.dispatches.iter_mut().enumerate() {
            if !d.due(now) {
                continue;
            }
            if d.dispatched_at.is_some() {
                out.push(if d.run.is_some() {
                    Due::Poll(i)
                } else {
                    Due::Find(i)
                });
                continue;
            }
            match mirror {
                Mirror::Passed => out.push(Due::Dispatch(i)),
                Mirror::Open => d.waiting = Some("the head isn't on GitHub yet".into()),
                Mirror::Failed => {
                    if !out.contains(&Due::Mirror) {
                        out.push(Due::Mirror);
                    }
                    d.fail(
                        "the head didn't reach GitHub; it is pushed again".into(),
                        now,
                    );
                }
            }
        }
        out
    }

    /// Entry `n` was dispatched at `now`; the checks it started.
    pub fn dispatched(&mut self, n: usize, now: u64) -> Vec<String> {
        let Some(d) = self.dispatches.get_mut(n) else {
            return Vec::new();
        };
        d.dispatched_at = Some(now);
        d.next_at = Some(now + POLL);
        d.failures = 0;
        d.waiting = None;
        d.checks
            .iter()
            .filter(|c| !d.reported.contains(c))
            .cloned()
            .collect()
    }

    /// A call for entry `n` failed at `now`: it is retried with backoff, its checks waiting.
    pub fn failed(&mut self, n: usize, why: &str, now: u64) {
        if let Some(d) = self.dispatches.get_mut(n) {
            d.fail(format!("waiting: GitHub ({why})"), now);
        }
    }

    /// Entry `n`'s run, looked for in the workflow's runs `listed`, at `now`. One not found within
    /// [`FIND_WITHIN`] is dispatched again, under a new ID.
    pub fn found(&mut self, n: usize, listed: &Value, now: u64) {
        let fresh = self.seq + 1;
        let Some(d) = self.dispatches.get_mut(n) else {
            return;
        };
        let suffix = title_suffix(&d.id);
        let run = listed["workflow_runs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| {
                r["display_title"]
                    .as_str()
                    .is_some_and(|t| t.ends_with(&suffix))
            });
        if let Some(r) = run
            && let Some(id) = r["id"].as_u64()
        {
            d.run = Some(id);
            d.url = r["html_url"].as_str().map(str::to_string);
            d.next_at = Some(now + POLL);
            d.failures = 0;
            d.waiting = None;
            return;
        }
        if d.dispatched_at.is_some_and(|t| now >= t + FIND_WITHIN) {
            let base =
                d.id.rsplit_once('-')
                    .map_or(d.id.as_str(), |(b, _)| b)
                    .to_string();
            d.id = format!("{base}-{fresh}");
            d.dispatched_at = None;
            d.fail(
                "waiting: GitHub (the run never appeared; dispatched again)".into(),
                now,
            );
            self.seq = fresh;
        } else {
            d.next_at = Some(now + POLL);
        }
    }

    /// Entry `n`'s run `run` and its `jobs`, polled at `now`: each check finished since.
    pub fn polled(&mut self, n: usize, run: &Value, jobs: &Value, now: u64) -> Vec<Finished> {
        let Some(d) = self.dispatches.get_mut(n) else {
            return Vec::new();
        };
        d.next_at = Some(now + POLL);
        d.failures = 0;
        d.waiting = None;
        if let Some(u) = run["html_url"].as_str() {
            d.url = Some(u.to_string());
        }
        let listed: Vec<&Value> = jobs["jobs"].as_array().into_iter().flatten().collect();
        let mut out = Vec::new();
        for check in d.checks.clone() {
            if d.reported.contains(&check) {
                continue;
            }
            let job = listed
                .iter()
                .find(|j| j["name"].as_str() == Some(check.as_str()));
            let result = match job {
                Some(j) if j["status"].as_str() == Some("completed") => Some(Finished {
                    name: check.clone(),
                    ok: j["conclusion"].as_str() == Some("success"),
                    job: j["id"].as_u64(),
                }),
                None if run["status"].as_str() == Some("completed") => Some(Finished {
                    name: check.clone(),
                    ok: false,
                    job: None,
                }),
                _ => None,
            };
            if let Some(f) = result {
                d.reported.push(check);
                out.push(f);
            }
        }
        out
    }

    /// What `change.status` shows: each entry's head, checks, run and wait.
    #[must_use]
    pub fn view(&self) -> Value {
        Value::Array(
            self.dispatches
                .iter()
                .map(|d| {
                    json!({
                        "sha": d.sha,
                        "dispatch": d.id,
                        "checks": d.checks,
                        "run": d.url,
                        "waiting": d.waiting,
                    })
                })
                .collect(),
        )
    }
}

/// The body that dispatches `main`'s [`WORKFLOW`] for entry `d` of change `number`, with the
/// checks' commands as `bound` recorded them.
#[must_use]
pub fn dispatch_body(d: &Dispatch, number: u32, checks: &[Check]) -> Value {
    let listed: Vec<&Check> = d
        .checks
        .iter()
        .filter(|c| !d.reported.contains(c))
        .filter_map(|c| checks.iter().find(|k| &k.name == c))
        .collect();
    json!({
        "ref": REF,
        "inputs": {
            "sha": d.sha,
            "number": number.to_string(),
            "dispatch": d.id,
            "checks": serde_json::to_string(&listed).unwrap_or_default(),
        },
    })
}
