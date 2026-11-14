//! An iOS release, as `MainRun` moves it on: a macOS provider archives it, the control plane
//! copies the archive to R2, Buildkite signs and uploads it, and App Store Connect processes it;
//! see the 2026-11-12 amendment to `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`
//! and `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §2.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::{self, Actions};
use crate::changes::Check;
use crate::control::Statement;
use crate::macos::{Job, Kind, Provider};

/// The check that archives a release, as `ci.yml` runs it.
pub const CHECK: &str = "ios.archive";
/// The run's artifact: the zipped archive, uploaded as is.
pub const ARTIFACT: &str = "Pigeon.xcarchive.zip";
/// A release's dispatches carry this plus its build number, so they never share an ID with the
/// test map's, which carry change 0.
pub const NUMBER_BASE: u32 = 1_000_000;
/// The Buildkite organization, pipeline and branch that sign.
pub const BUILDKITE_ORG: &str = crate::apple::ORGANIZATION;
/// The signing pipeline.
pub const BUILDKITE_PIPELINE: &str = crate::apple::PIPELINE;

/// Where release `build` of `sha` is kept.
#[must_use]
pub fn key(sha: &str, build: u64) -> String {
    format!("builds/{sha}/ios/{build}.xcarchive.zip")
}

/// The archive check of `build`.
#[must_use]
pub fn check(build: u64) -> Check {
    Check {
        name: CHECK.into(),
        argv: vec![
            "bash".into(),
            actions::IOS_ARCHIVE.into(),
            build.to_string(),
        ],
    }
}

/// The dispatch number of `build`.
#[must_use]
pub fn number(build: u64) -> u32 {
    NUMBER_BASE + u32::try_from(build).unwrap_or(0)
}

/// Where a release is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// A macOS provider archives it.
    Archiving,
    /// Its archive is copied to R2, then its Buildkite build created.
    Copying,
    /// Buildkite signs and uploads it.
    Signing,
    /// App Store Connect processes it.
    Processing,
}

/// The release `MainRun` is moving on; one at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Current {
    /// Its build number.
    pub build: u64,
    /// The commit of `main` it releases.
    pub sha: String,
    /// Where it is.
    pub stage: Stage,
    /// The GitHub run that archived it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<u64>,
    /// The provider that archived it, and its run or build there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<(Provider, String)>,
    /// Its Buildkite build's number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buildkite: Option<u64>,
    /// When it last moved, in Unix seconds.
    pub since: u64,
}

/// `MainRun`'s iOS releases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IosReleases {
    /// The archive's GitHub dispatch.
    #[serde(default)]
    pub actions: Actions,
    /// The release in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Current>,
    /// Its archive's macOS job, across providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<Job>,
}

/// What became of the release in flight, for D1 and the event log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    /// It moved to `state`.
    To(&'static str),
    /// It failed, for this reason.
    Failed(String),
}

/// The commit a release archives: `main`'s head, once GitHub holds it. Refused while the mirror
/// is behind, so a release never takes an older commit; see the release ADR's 2026-11-12
/// amendment on the release's commit.
///
/// # Errors
///
/// When nothing is mirrored, or `main` is ahead of what GitHub holds.
pub fn release_sha(target: Option<&str>, mirrored: Option<&str>) -> Result<String, String> {
    let mirrored = mirrored.ok_or("main has not reached GitHub yet, so nothing can be archived")?;
    match target {
        Some(t) if t != mirrored => Err(format!(
            "main is at {} but GitHub holds {}; release once the mirror catches up",
            &t[..t.len().min(12)],
            &mirrored[..mirrored.len().min(12)]
        )),
        _ => Ok(mirrored.to_string()),
    }
}

impl IosReleases {
    /// Starts release `build` of `sha`, unless one is in flight.
    ///
    /// # Errors
    ///
    /// When a release is in flight.
    pub fn start(&mut self, build: u64, sha: &str, now: u64) -> Result<(), String> {
        if let Some(c) = &self.current {
            return Err(format!(
                "release {} is still {}",
                c.build,
                stage_name(c.stage)
            ));
        }
        self.actions = Actions::default();
        self.macos = Some(Job::new(Kind::Archive, build, sha, now, None, Vec::new()));
        self.current = Some(Current {
            build,
            sha: sha.into(),
            stage: Stage::Archiving,
            run: None,
            archive: None,
            buildkite: None,
            since: now,
        });
        Ok(())
    }

    /// The archive was placed on GitHub: one dispatch of [`CHECK`], afresh.
    pub fn on_github(&mut self) {
        let Some(c) = &self.current else {
            return;
        };
        self.actions = Actions::default();
        self.actions.queue(number(c.build), &c.sha, CHECK);
    }

    /// Whether the alarm should come back soon.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.current.is_some()
    }

    /// The archive passed on `provider`, as its run or build `id`: it is copied next.
    pub fn archived_on(&mut self, provider: Provider, id: &str, now: u64) -> Moved {
        let Some(c) = self.current.as_mut() else {
            return Moved::Failed("no release is in flight".into());
        };
        c.stage = Stage::Copying;
        c.run = (provider == Provider::Github)
            .then(|| id.parse().ok())
            .flatten();
        c.archive = Some((provider, id.into()));
        c.since = now;
        Moved::To("archiving")
    }

    /// The archive was copied and Buildkite build `number` created.
    pub fn signing(&mut self, buildkite: u64, now: u64) -> Moved {
        if let Some(c) = self.current.as_mut() {
            c.stage = Stage::Signing;
            c.buildkite = Some(buildkite);
            c.since = now;
        }
        Moved::To("archived")
    }

    /// Buildkite's build is in `state`, and the release's D1 row in `row_state`.
    pub fn signed(&mut self, state: &str, row_state: &str, now: u64) -> Option<Moved> {
        let moved = buildkite_outcome(state, row_state)?;
        match &moved {
            Moved::To(_) => {
                if let Some(c) = self.current.as_mut() {
                    c.stage = Stage::Processing;
                    c.since = now;
                }
            }
            Moved::Failed(_) => self.current = None,
        }
        Some(moved)
    }

    /// App Store Connect's `processingState` for the build; none while it is unknown or still
    /// processing.
    pub fn processed(&mut self, state: Option<&str>) -> Option<Moved> {
        let moved = match state? {
            "VALID" => Moved::To("delivered"),
            "INVALID" | "FAILED" => {
                Moved::Failed(format!("App Store Connect found the build {}", state?))
            }
            _ => return None,
        };
        self.current = None;
        Some(moved)
    }

    /// Ends the release in flight, for `why`.
    pub fn fail(&mut self, why: &str) -> Moved {
        self.current = None;
        Moved::Failed(why.into())
    }

    /// What `ios.status` shows of the release in flight.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "current": self.current.as_ref().map(|c| json!({
                "build": c.build,
                "sha": c.sha,
                "stage": stage_name(c.stage),
                "run": c.run,
                "archive": c.archive.as_ref().map(|(p, id)| json!({ "provider": p.name(), "id": id })),
                "buildkite": c.buildkite,
                "since": c.since,
            })),
            "dispatches": self.actions.view(),
            "macos": self.macos.as_ref().map(crate::macos::view),
        })
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Archiving => "archiving",
        Stage::Copying => "copying",
        Stage::Signing => "signing",
        Stage::Processing => "processing",
    }
}

/// A finished Buildkite build's state, against its release row: passed after the `sign` step
/// redeemed the grant is `uploaded`; passed without a redemption, failed or canceled fails; any
/// other state waits.
#[must_use]
pub fn buildkite_outcome(state: &str, row_state: &str) -> Option<Moved> {
    match state {
        "passed" if row_state == "signing" => Some(Moved::To("uploaded")),
        "passed" => Some(Moved::Failed(
            "Buildkite passed but the sign step never redeemed its grant".into(),
        )),
        "failed" | "canceled" | "canceling" | "not_run" | "skipped" => {
            Some(Moved::Failed(format!("Buildkite's build {state}")))
        }
        _ => None,
    }
}

/// The body that creates release `build`'s Buildkite build of `sha`.
#[must_use]
pub fn buildkite_body(build: u64, sha: &str) -> Value {
    json!({
        "commit": sha,
        "branch": crate::apple::BRANCH,
        "message": format!("pigeon iOS build {build}"),
        "env": { "PIGEON_IOS_BUILD": build.to_string() },
    })
}

/// The path of the Buildkite builds of the signing pipeline, or of one build.
#[must_use]
pub fn buildkite_path(build: Option<u64>) -> String {
    let base = format!("/v2/organizations/{BUILDKITE_ORG}/pipelines/{BUILDKITE_PIPELINE}/builds");
    build.map_or(base.clone(), |n| format!("{base}/{n}"))
}

/// The next build number, after the highest in `ios_releases`.
#[must_use]
pub fn next_build() -> Statement {
    Statement {
        sql: "SELECT COALESCE(MAX(build), 0) + 1 AS build FROM ios_releases",
        params: vec![],
    }
}

/// Inserts release `build` of `sha`, archiving.
#[must_use]
pub fn insert(build: u64, sha: &str, now: u64) -> Statement {
    Statement {
        sql: "INSERT INTO ios_releases (build, sha, state, created_at, updated_at) \
              VALUES (?1, ?2, 'archiving', ?3, ?3)",
        params: vec![json!(build), json!(sha), json!(now)],
    }
}

/// Moves release `build` to `state`, recording its Buildkite build when known.
#[must_use]
pub fn set_state(build: u64, state: &str, buildkite: Option<u64>, now: u64) -> Statement {
    Statement {
        sql: "UPDATE ios_releases SET state = ?2, buildkite_build = COALESCE(?3, buildkite_build), \
              updated_at = ?4 WHERE build = ?1",
        params: vec![json!(build), json!(state), json!(buildkite), json!(now)],
    }
}

/// Records where release `build` was archived: the provider, and its run or build there.
#[must_use]
pub fn set_archive(build: u64, provider: Provider, id: &str, now: u64) -> Statement {
    Statement {
        sql: "UPDATE ios_releases SET archive_provider = ?2, archive_build = ?3, updated_at = ?4 \
              WHERE build = ?1",
        params: vec![json!(build), json!(provider.name()), json!(id), json!(now)],
    }
}

/// The state of release `build`.
#[must_use]
pub fn state_of(build: u64) -> Statement {
    Statement {
        sql: "SELECT state FROM ios_releases WHERE build = ?1",
        params: vec![json!(build)],
    }
}

/// The newest releases.
#[must_use]
pub fn recent() -> Statement {
    Statement {
        sql: "SELECT build, sha, buildkite_build, state, error, created_at, updated_at \
              FROM ios_releases ORDER BY build DESC LIMIT 10",
        params: vec![],
    }
}

/// The archive's row for a redeemed `sign` step: its build number, when that build and commit
/// are signing.
#[must_use]
pub fn signing_row(buildkite: u64, sha: &str) -> Statement {
    Statement {
        sql: "SELECT build FROM ios_releases WHERE buildkite_build = ?1 AND sha = ?2 \
              AND state = 'signing'",
        params: vec![json!(buildkite), json!(sha)],
    }
}

/// App Store Connect's processing state of a `GET /v1/builds?filter[version]=…` answer.
#[must_use]
pub fn processing_state(answer: &Value) -> Option<&str> {
    answer["data"][0]["attributes"]["processingState"].as_str()
}

/// Why `body` is not a release's zipped archive; none when it is.
#[must_use]
pub fn invalid(body: &[u8]) -> Option<String> {
    if !body.starts_with(b"PK\x03\x04") {
        return Some("the artifact is not a zip".into());
    }
    if body.len() < 1024 {
        return Some(format!(
            "the artifact is {} bytes, too small for an archive",
            body.len()
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_waits_for_the_mirror() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        assert_eq!(release_sha(Some(&a), Some(&a)).as_deref(), Ok(a.as_str()));
        assert_eq!(release_sha(None, Some(&a)).as_deref(), Ok(a.as_str()));
        let behind = release_sha(Some(&b), Some(&a)).expect_err("main is ahead");
        assert!(
            behind.contains("bbbbbbbbbbbb") && behind.contains("aaaaaaaaaaaa"),
            "{behind}"
        );
        assert!(release_sha(Some(&a), None).is_err());
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_release_moves_from_archive_to_delivery() {
        let mut r = IosReleases::default();
        r.start(3, SHA, 10).expect("starts");
        assert!(r.actions.dispatches.is_empty(), "nothing until placed");
        assert_eq!(r.macos.as_ref().map(|j| j.number), Some(3));
        r.on_github();
        assert_eq!(r.actions.dispatches.len(), 1);
        assert!(r.actions.dispatches[0].id.starts_with("1000003-"));
        assert_eq!(
            r.archived_on(Provider::Github, "43316598157", 20),
            Moved::To("archiving")
        );
        let c = r.current.as_ref().expect("in flight");
        assert_eq!(c.stage, Stage::Copying);
        assert_eq!(c.run, Some(43_316_598_157));
        let mut b = IosReleases::default();
        b.start(4, SHA, 10).expect("starts");
        b.archived_on(Provider::Bitrise, "slug-1", 20);
        let c = b.current.as_ref().expect("in flight");
        assert_eq!(
            (c.run, c.archive.clone()),
            (None, Some((Provider::Bitrise, "slug-1".into())))
        );
        assert_eq!(r.signing(77, 30), Moved::To("archived"));
        assert_eq!(r.current.as_ref().and_then(|c| c.buildkite), Some(77));
        assert_eq!(r.signed("running", "signing", 40), None);
        assert_eq!(
            r.signed("passed", "signing", 50),
            Some(Moved::To("uploaded"))
        );
        assert_eq!(r.processed(Some("PROCESSING")), None);
        assert_eq!(r.processed(None), None);
        assert_eq!(r.processed(Some("VALID")), Some(Moved::To("delivered")));
        assert!(!r.busy());
    }

    #[test]
    fn one_release_at_a_time() {
        let mut r = IosReleases::default();
        r.start(1, SHA, 0).expect("starts");
        assert!(r.start(2, SHA, 0).is_err());
    }

    #[test]
    fn every_failure_ends_the_release() {
        let mut r = IosReleases::default();
        r.start(1, SHA, 0).expect("starts");
        assert!(matches!(
            r.fail("github's archive failed"),
            Moved::Failed(_)
        ));
        assert!(!r.busy());

        for (state, row) in [
            ("passed", "archived"),
            ("failed", "signing"),
            ("canceled", "archived"),
        ] {
            let mut r = IosReleases::default();
            r.start(1, SHA, 0).expect("starts");
            r.signing(5, 1);
            assert!(
                matches!(r.signed(state, row, 2), Some(Moved::Failed(_))),
                "{state} {row}"
            );
            assert!(!r.busy());
        }

        let mut r = IosReleases::default();
        r.start(1, SHA, 0).expect("starts");
        assert!(matches!(
            r.processed(Some("INVALID")),
            Some(Moved::Failed(_))
        ));
    }

    #[test]
    fn the_archive_check_and_its_buildkite_build_name_the_build() {
        assert_eq!(check(12).argv, ["bash", actions::IOS_ARCHIVE, "12"]);
        assert!(actions::on_github(&check(12).argv));
        assert_eq!(key(SHA, 12), format!("builds/{SHA}/ios/12.xcarchive.zip"));
        let body = buildkite_body(12, SHA);
        assert_eq!(body["commit"], SHA);
        assert_eq!(body["branch"], "main");
        assert_eq!(body["env"]["PIGEON_IOS_BUILD"], "12");
        assert_eq!(
            buildkite_path(Some(9)),
            "/v2/organizations/willow-graysen/pipelines/pigeon-ios-release/builds/9"
        );
    }

    #[test]
    fn only_a_zip_of_some_size_is_an_archive() {
        assert!(invalid(b"{}").is_some());
        assert!(invalid(b"PK\x03\x04tiny").is_some());
        let mut zip = b"PK\x03\x04".to_vec();
        zip.resize(4096, 0);
        assert!(invalid(&zip).is_none());
    }

    #[test]
    fn processing_state_reads_the_first_build() {
        let answer = json!({ "data": [{ "attributes": { "processingState": "VALID" } }] });
        assert_eq!(processing_state(&answer), Some("VALID"));
        assert_eq!(processing_state(&json!({ "data": [] })), None);
    }
}
