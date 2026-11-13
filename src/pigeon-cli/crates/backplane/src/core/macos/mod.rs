//! Where a macOS job runs and when it moves: `.config/macos.yaml`, each provider's outcome, the
//! verdict on it, provider health, and the next provider; pure. See
//! `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §1, §2 and §5.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

/// The reviewed order, start time and caps.
pub const CONFIG: &str = include_str!("../../../../../../../.config/macos.yaml");

/// Dispatches that fail in a row before a provider is `refused`.
pub const DISPATCH_TRIES: u32 = 3;
/// Moves away from a provider within [`WINDOW`] that mark it down.
pub const MOVES_DOWN: usize = 2;
/// The window [`MOVES_DOWN`] counts in, in seconds.
pub const WINDOW: u64 = 30 * 60;
/// How long a provider is down before a job probes it, in seconds.
pub const PROBE_AFTER: u64 = 15 * 60;
/// The longest a provider stays down before the next probe, in seconds.
pub const MAX_WAIT: u64 = 2 * 60 * 60;
/// How often a job with no provider is offered again, from the start of the order, in seconds.
pub const WAIT_RETRY: u64 = 15 * 60;
/// How long a release's archive may wait for a provider before the release fails, in seconds.
pub const GIVE_UP: u64 = 2 * 60 * 60;

/// A provider of macOS machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// GitHub Actions' hosted `macos-15`.
    Github,
    /// Bitrise's hosted macOS.
    Bitrise,
    /// Buildkite's hosted macOS, pipeline `pigeon-ios-build`.
    Buildkite,
}

impl Provider {
    /// Every provider.
    pub const ALL: [Self; 3] = [Self::Github, Self::Bitrise, Self::Buildkite];

    /// Its name in config, events and rows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Bitrise => "bitrise",
            Self::Buildkite => "buildkite",
        }
    }

    /// The provider named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// What a macOS job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A change head's `ios` check.
    Check,
    /// A release's `ios.archive`.
    Archive,
}

impl Kind {
    /// Its name in rows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Archive => "archive",
        }
    }
}

/// `.config/macos.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Providers, first available first.
    pub order: Vec<Provider>,
    /// How long a job may wait for a machine, in seconds.
    pub start_within: u64,
    /// Each provider's monthly minutes; none is no cap.
    pub caps: BTreeMap<Provider, Option<u64>>,
}

impl Config {
    /// `p`'s monthly cap, if it has one.
    #[must_use]
    pub fn cap(&self, p: Provider) -> Option<u64> {
        self.caps.get(&p).copied().flatten()
    }
}

/// The built-in [`CONFIG`].
///
/// # Errors
///
/// When it doesn't parse, which its tests rule out.
pub fn config() -> Result<Config, String> {
    parse(CONFIG)
}

/// Parses `.config/macos.yaml`: `order`, `start_within_minutes` and each provider's
/// `<name>_monthly_minutes`, and nothing else.
///
/// # Errors
///
/// When a key is unknown, missing or repeated, or a value is not what it should be.
pub fn parse(yaml: &str) -> Result<Config, String> {
    let mut order = None;
    let mut start_within = None;
    let mut caps = BTreeMap::new();
    for line in yaml.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .map(|(k, v)| (k.trim(), v.trim()))
            .ok_or_else(|| format!("`{line}` is not `key: value`"))?;
        if key == "order" {
            let list = value
                .strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .ok_or("`order` is not a [list]")?;
            let mut ps = Vec::new();
            for name in list.split(',').map(str::trim) {
                let p = Provider::parse(name).ok_or_else(|| format!("no provider `{name}`"))?;
                if ps.contains(&p) {
                    return Err(format!("`{name}` is in `order` twice"));
                }
                ps.push(p);
            }
            if order.replace(ps).is_some() {
                return Err("`order` is repeated".into());
            }
        } else if key == "start_within_minutes" {
            let m: u64 = value
                .parse()
                .ok()
                .filter(|&m| m > 0)
                .ok_or("`start_within_minutes` is not a whole number above 0")?;
            if start_within.replace(m * 60).is_some() {
                return Err("`start_within_minutes` is repeated".into());
            }
        } else if let Some(p) = key
            .strip_suffix("_monthly_minutes")
            .and_then(Provider::parse)
        {
            let cap = match value {
                "null" => None,
                v => Some(
                    v.parse()
                        .map_err(|_| format!("`{key}` is not a whole number or null"))?,
                ),
            };
            if caps.insert(p, cap).is_some() {
                return Err(format!("`{key}` is repeated"));
            }
        } else {
            return Err(format!("unknown key `{key}`"));
        }
    }
    let order = order.ok_or("`order` is missing")?;
    if order.is_empty() {
        return Err("`order` is empty".into());
    }
    if let Some(p) = Provider::ALL.into_iter().find(|p| !caps.contains_key(p)) {
        return Err(format!("`{}_monthly_minutes` is missing", p.name()));
    }
    Ok(Config {
        order,
        start_within: start_within.ok_or("`start_within_minutes` is missing")?,
        caps,
    })
}

/// How an attempt on a provider stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// No machine yet, within `start_within`.
    Waiting,
    /// A machine has it.
    Running,
    /// It ran and passed.
    Passed,
    /// It ran and failed: a result, never a reason to move.
    Failed,
    /// No machine took it within `start_within`, or it was cancelled before one did.
    Unassigned,
    /// Its machine went away before our step finished.
    Lost,
    /// The provider would not take it.
    Refused,
}

impl Outcome {
    /// Its name in rows and events.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Unassigned => "unassigned",
            Self::Lost => "lost",
            Self::Refused => "refused",
        }
    }

    /// It ran to a result.
    #[must_use]
    pub fn ran(self) -> bool {
        matches!(self, Self::Passed | Self::Failed)
    }

    /// Nothing more will happen to it.
    #[must_use]
    pub fn ended(self) -> bool {
        !matches!(self, Self::Waiting | Self::Running)
    }
}

fn late(queued_at: u64, now: u64, start_within: u64) -> bool {
    now.saturating_sub(queued_at) >= start_within
}

fn unstarted(queued_at: u64, now: u64, start_within: u64) -> Outcome {
    if late(queued_at, now, start_within) {
        Outcome::Unassigned
    } else {
        Outcome::Waiting
    }
}

/// A GitHub job's outcome, from its entry in the run's `jobs`; `step` is our step's name.
#[must_use]
pub fn github(job: &Value, step: &str, queued_at: u64, now: u64, start_within: u64) -> Outcome {
    let runner = job["runner_id"].as_u64().filter(|&id| id > 0);
    if job["status"] != "completed" {
        return if runner.is_some() {
            Outcome::Running
        } else {
            unstarted(queued_at, now, start_within)
        };
    }
    if runner.is_none() {
        return Outcome::Unassigned;
    }
    match job["conclusion"].as_str().unwrap_or_default() {
        "success" => Outcome::Passed,
        "cancelled" => {
            let ours = job["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["name"] == step && s["status"] == "completed");
            match ours.and_then(|s| s["conclusion"].as_str()) {
                Some("success") => Outcome::Passed,
                Some("failure") => Outcome::Failed,
                _ => Outcome::Lost,
            }
        }
        _ => Outcome::Failed,
    }
}

/// A Bitrise build's outcome, from `GET /apps/{app}/builds/{build}`'s `data`.
#[must_use]
pub fn bitrise(build: &Value, queued_at: u64, now: u64, start_within: u64) -> Outcome {
    let started = build["started_on_worker_at"].is_string();
    match build["status"].as_u64() {
        Some(1) => Outcome::Passed,
        Some(2) => Outcome::Failed,
        Some(3 | 4) if started => Outcome::Lost,
        Some(3 | 4) => Outcome::Unassigned,
        _ if started => Outcome::Running,
        _ if build["is_on_hold"] == true && late(queued_at, now, start_within) => Outcome::Refused,
        _ => unstarted(queued_at, now, start_within),
    }
}

/// A Buildkite job's outcome, from its entry in the build's `jobs`.
#[must_use]
pub fn buildkite(job: &Value, queued_at: u64, now: u64, start_within: u64) -> Outcome {
    let started = job["started_at"].is_string();
    let agent_lost =
        job["exit_status"].as_i64() == Some(-1) || job["signal_reason"] == "agent_lost";
    match job["state"].as_str().unwrap_or_default() {
        "passed" => Outcome::Passed,
        "failed" if agent_lost => Outcome::Lost,
        "failed" => Outcome::Failed,
        "timed_out" if started => Outcome::Failed,
        "canceled" if started => Outcome::Lost,
        "timed_out" | "canceled" | "expired" => Outcome::Unassigned,
        "broken" => Outcome::Refused,
        "running" | "assigned" | "accepted" | "canceling" => Outcome::Running,
        _ if started => Outcome::Running,
        _ => unstarted(queued_at, now, start_within),
    }
}

/// Whether `failures` dispatches in a row mean the provider refused.
#[must_use]
pub fn refused(failures: u32) -> bool {
    failures >= DISPATCH_TRIES
}

/// What to do about an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Keep polling.
    Wait,
    /// The job ran to this result.
    Done {
        /// It passed.
        passed: bool,
    },
    /// Try the next provider.
    Move {
        /// Why, for `ci.moved` and the digest.
        why: String,
    },
    /// The job fails without running to a result.
    Fail {
        /// Why.
        why: String,
    },
}

/// The verdict on `outcome` from `provider`, given whether this job lost a machine before.
#[must_use]
pub fn verdict(provider: Provider, outcome: Outcome, lost_before: bool) -> Verdict {
    let p = provider.name();
    match outcome {
        Outcome::Waiting | Outcome::Running => Verdict::Wait,
        Outcome::Passed => Verdict::Done { passed: true },
        Outcome::Failed => Verdict::Done { passed: false },
        Outcome::Lost if lost_before => Verdict::Fail {
            why: format!("{p} lost its machine, the second lost for this commit"),
        },
        Outcome::Lost => Verdict::Move {
            why: format!("{p} lost its machine"),
        },
        Outcome::Unassigned => Verdict::Move {
            why: format!("{p} gave no machine"),
        },
        Outcome::Refused => Verdict::Move {
            why: format!("{p} refused the job"),
        },
    }
}

/// When a provider is down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Down {
    /// Since when.
    pub since: u64,
    /// No job before this, except a probe after it.
    pub until: u64,
    /// The current wait, doubling up to [`MAX_WAIT`].
    pub wait: u64,
    /// A probe is out.
    #[serde(default)]
    pub probing: bool,
}

/// A provider's health, kept on `Instance`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Moves away from it within [`WINDOW`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<u64>,
    /// When it is down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<Down>,
}

/// Every provider's health.
pub type Health = BTreeMap<Provider, ProviderHealth>;

/// Each provider's minutes this month.
pub type Usage = BTreeMap<Provider, u64>;

/// Whether a provider takes a job now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// It is healthy.
    Up,
    /// It is down, and the next job probes it alone.
    Probe,
    /// It is down until then, or a probe is out.
    Down {
        /// When a job may probe it.
        until: u64,
    },
}

/// `h` at `now`.
#[must_use]
pub fn availability(h: &ProviderHealth, now: u64) -> Availability {
    match &h.down {
        None => Availability::Up,
        Some(d) if now >= d.until && !d.probing => Availability::Probe,
        Some(d) => Availability::Down { until: d.until },
    }
}

/// `h` after a job moved away from it at `now`: down at the [`MOVES_DOWN`]th move in [`WINDOW`],
/// and a failed probe doubles the wait.
#[must_use]
pub fn moved_away(h: &ProviderHealth, now: u64) -> ProviderHealth {
    match &h.down {
        Some(d) if d.probing => ProviderHealth {
            moves: Vec::new(),
            down: Some(Down {
                since: d.since,
                until: now + (d.wait * 2).min(MAX_WAIT),
                wait: (d.wait * 2).min(MAX_WAIT),
                probing: false,
            }),
        },
        Some(_) => h.clone(),
        None => {
            let mut moves: Vec<u64> = h
                .moves
                .iter()
                .copied()
                .filter(|&t| now.saturating_sub(t) < WINDOW)
                .collect();
            moves.push(now);
            if moves.len() >= MOVES_DOWN {
                ProviderHealth {
                    moves: Vec::new(),
                    down: Some(Down {
                        since: now,
                        until: now + PROBE_AFTER,
                        wait: PROBE_AFTER,
                        probing: false,
                    }),
                }
            } else {
                ProviderHealth { moves, down: None }
            }
        }
    }
}

/// `h` once a job ran on it: healthy.
#[must_use]
pub fn ran(h: &ProviderHealth) -> ProviderHealth {
    ProviderHealth {
        moves: h.moves.clone(),
        down: None,
    }
}

/// `h` once a job was sent to probe it.
#[must_use]
pub fn probing(h: &ProviderHealth) -> ProviderHealth {
    ProviderHealth {
        moves: h.moves.clone(),
        down: h.down.as_ref().map(|d| Down {
            probing: true,
            ..d.clone()
        }),
    }
}

/// Where a job goes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// To this provider; `probe` when it is down and this job probes it.
    Run {
        /// The provider.
        provider: Provider,
        /// It probes a down provider.
        probe: bool,
    },
    /// Nowhere yet: `why` is `macOS (<each provider and why>)`, offered again after
    /// [`WAIT_RETRY`].
    Wait {
        /// Why, for `waiting:`.
        why: String,
    },
}

/// The first provider in `cfg`'s order that is not `tried`, under its cap and healthy (or due a
/// probe).
#[must_use]
pub fn next(cfg: &Config, health: &Health, usage: &Usage, tried: &[Provider], now: u64) -> Next {
    let mut why = Vec::new();
    for &p in &cfg.order {
        if tried.contains(&p) {
            why.push(format!("{}: tried", p.name()));
            continue;
        }
        let used = usage.get(&p).copied().unwrap_or(0);
        if let Some(cap) = cfg.cap(p).filter(|&c| used >= c) {
            why.push(format!("{}: {used} of {cap} minutes this month", p.name()));
            continue;
        }
        match availability(&health.get(&p).cloned().unwrap_or_default(), now) {
            Availability::Up => {
                return Next::Run {
                    provider: p,
                    probe: false,
                };
            }
            Availability::Probe => {
                return Next::Run {
                    provider: p,
                    probe: true,
                };
            }
            Availability::Down { until } => why.push(format!(
                "{}: down for {} more minutes",
                p.name(),
                until.saturating_sub(now).div_ceil(60).max(1)
            )),
        }
    }
    Next::Wait {
        why: format!("macOS ({})", why.join("; ")),
    }
}

/// What a new `ios` check counts as tried: GitHub while the CI window is open.
#[must_use]
pub fn tried_for_window(window_open: bool) -> Vec<Provider> {
    if window_open {
        vec![Provider::Github]
    } else {
        Vec::new()
    }
}

/// Whole minutes from `started` to `finished`, rounded up.
#[must_use]
pub fn minutes(started: u64, finished: u64) -> u64 {
    finished.saturating_sub(started).div_ceil(60)
}

/// Whether `used` is past 80% of `cap`.
#[must_use]
pub fn near_cap(used: u64, cap: Option<u64>) -> bool {
    cap.is_some_and(|c| used.saturating_mul(5) >= c.saturating_mul(4))
}

/// The start of the UTC month holding `t`, in Unix seconds.
#[must_use]
pub fn month_start(t: u64) -> u64 {
    let days = t / 86_400;
    let (y, m, _) = civil(days);
    days_from_civil(y, m, 1) * 86_400
}

// Howard Hinnant's civil-from-days and its inverse, for days since 1970-01-01.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

fn days_from_civil(y: u64, m: u64, d: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The SHA-256 of `text` with line endings and outer whitespace normalized, in hex.
#[must_use]
pub fn config_digest(text: &str) -> String {
    let normal = text.replace("\r\n", "\n");
    Sha256::digest(normal.trim().as_bytes())
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Whether the config a provider holds is `ours`, `main`'s.
#[must_use]
pub fn same_config(held: &str, ours: &str) -> bool {
    config_digest(held) == config_digest(ours)
}
