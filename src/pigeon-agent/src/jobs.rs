//! A job Container's work: `pigeon agent listen` takes a [`JobSpec`] over the signed link,
//! checks out its head, runs it in the background, and reports; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §3, §4.
//!
//! The bound job is the judge's change policy and `change.scope` on the checked-out tree, with
//! the change's approved scope; every other job is a command `change.scope` named.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use crate::checks::scope::ReadTestMap;
use crate::control::{
    DEPLOY, JobSpec, MIRROR, MIRROR_TO, MIRROR_TOKEN, Outcome, Report, UNMIRROR, tail,
};
use crate::core::change::{BOUND, Check, ConflictContext, MERGE, RESTACK, Relation, Restacked};
use crate::release::LoadConfig;
use crate::{Instance, Request, run_with};

/// Where jobs check out, under the Container's temporary directory.
const WORK: &str = "pigeon-jobs";

/// The jobs this Container runs, by ID.
#[derive(Debug, Default)]
pub struct Board {
    reports: Mutex<HashMap<String, Report>>,
    work: Option<PathBuf>,
}

impl Board {
    /// A board checking out under `work`, else the temporary directory.
    #[must_use]
    pub fn new(work: Option<PathBuf>) -> Self {
        Self {
            reports: Mutex::new(HashMap::new()),
            work,
        }
    }

    fn set(&self, report: Report) {
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(report.id.clone(), report);
    }

    /// Job `id`'s report, if it was started here.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Report> {
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Starts `spec` in the background, unless it is already here; its report so far.
    pub fn start(self: &Arc<Self>, spec: JobSpec) -> Report {
        if let Some(r) = self.get(&spec.id) {
            return r;
        }
        let running = Report {
            id: spec.id.clone(),
            outcome: Outcome::Running,
            log: String::new(),
            violations: Vec::new(),
            checks: Vec::new(),
            full: Vec::new(),
            gate: None,
            patch: None,
            own: None,
            merged: None,
            published: Vec::new(),
            ios: false,
            workers: Vec::new(),
            passed: Vec::new(),
            build: None,
            images: None,
            tiers: None,
            restack: None,
            platform: None,
            evictions: Vec::new(),
            forecast: Vec::new(),
            built_on: None,
            ledger: None,
            pool: None,
            serving: None,
            pipeline: false,
            standby: None,
        };
        self.set(running.clone());
        let board = Arc::clone(self);
        std::thread::spawn(move || {
            let dir = board
                .work
                .clone()
                .unwrap_or_else(std::env::temp_dir)
                .join(WORK)
                .join(dir_name(&spec));
            let report = run(&spec, &dir);
            let _ = std::fs::remove_dir_all(&dir);
            board.set(report);
        });
        running
    }
}

/// The directory a job checks out into: its ID, but a platform apply's is its plan's, since the
/// saved plan carries the state's absolute path; see
/// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md` (step 4c-1).
#[must_use]
pub fn dir_name(spec: &JobSpec) -> String {
    match spec.id.strip_suffix("-apply") {
        Some(run) if spec.job == crate::control::PLATFORM => format!("{run}-plan"),
        _ => spec.id.clone(),
    }
}

pub(crate) fn report(spec: &JobSpec, outcome: Outcome, log: &str) -> Report {
    Report {
        id: spec.id.clone(),
        outcome,
        log: tail(log).to_string(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: Vec::new(),
        gate: None,
        patch: None,
        own: None,
        merged: None,
        published: Vec::new(),
        ios: false,
        workers: Vec::new(),
        passed: Vec::new(),
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
        evictions: Vec::new(),
        forecast: Vec::new(),
        built_on: None,
        ledger: None,
        pool: None,
        serving: None,
        pipeline: false,
        standby: None,
    }
}

/// Runs `spec` in `dir`: checks out its head, then the bound job or its command; the bound job
/// judges with the `pigeon-eviction` [`crate::eviction::locate`] finds.
#[must_use]
pub fn run(spec: &JobSpec, dir: &Path) -> Report {
    run_judged(
        spec,
        dir,
        &crate::eviction::locate(&|k| std::env::var(k).ok()),
    )
}

/// [`run`], the bound job judging with `judge`.
#[must_use]
pub fn run_judged(spec: &JobSpec, dir: &Path, judge: &Path) -> Report {
    // A merge whose report was lost may already be on the base; finding it there is the merge.
    if spec.job == MERGE
        && let Ok(Some(commit)) =
            crate::checks::ci::merged_already(dir, &spec.remote, &spec.token, &spec.base, &spec.sha)
    {
        return Report {
            log: format!("{} is already on {} as {commit}\n", spec.sha, spec.base),
            merged: Some(commit),
            ..report(spec, Outcome::Passed, "")
        };
    }
    if spec.job == MIRROR || spec.job == UNMIRROR {
        return mirror(spec, dir);
    }
    if spec.job == crate::control::UNSCRATCH {
        return unscratch(spec, dir);
    }
    if spec.job == crate::control::RELEASE {
        return release(spec, dir);
    }
    if spec.job == crate::control::POOL {
        return crate::pool_job::run(spec, dir);
    }
    let _ = std::fs::remove_dir_all(dir);
    if spec.job == crate::control::PLATFORM && !crate::standup_job::asks(&spec.argv) {
        return platform(spec, dir);
    }
    if let Err(e) = crate::checks::ci::checkout(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
    ) {
        if spec.job == DEPLOY
            && let Some(why) = crate::ship::superseded(spec, dir)
        {
            return Report {
                violations: vec![why],
                ..report(spec, Outcome::Superseded, &e.to_string())
            };
        }
        let mut r = report(spec, Outcome::Failed, &e.to_string());
        if spec.job == BOUND {
            r.violations = vec![e.to_string()];
        }
        return r;
    }
    if spec.job == BOUND {
        return bound(spec, dir, judge);
    }
    if spec.job == crate::control::EVICTION {
        return eviction(spec, dir, judge);
    }
    if spec.job == MERGE {
        return merge(spec, dir);
    }
    if spec.job == RESTACK {
        return restack(spec, dir);
    }
    if spec.job == DEPLOY {
        return deploy(spec, dir);
    }
    if spec.job == crate::control::PLATFORM {
        return crate::platform_job::run(spec, dir);
    }
    // A check may read a released template by its tag, as `sites.build` does.
    if let Err(e) = crate::checks::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return report(spec, Outcome::Failed, &e.to_string());
    }
    let (ok, log) =
        crate::checks::ci::exec(dir, &spec.argv, crate::checks::ci::TIMEOUT, &spec.token);
    report(
        spec,
        if ok { Outcome::Passed } else { Outcome::Failed },
        &log,
    )
}

/// A platform job: a plan checks out `main`'s head as it starts, an apply the commit its plan
/// recorded, which must be on `main`; its result names the commit. See
/// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`'s 2026-11-17 amendment.
fn platform(spec: &JobSpec, dir: &Path) -> Report {
    let pinned = match crate::platform_job::Task::parse(&spec.argv) {
        Ok(crate::platform_job::Task::Apply { .. }) => Some(spec.sha.as_str()),
        _ => None,
    };
    match crate::checks::ci::checkout_on(dir, &spec.remote, &spec.token, &spec.branch, pinned) {
        Ok(sha) => {
            let mut r = crate::platform_job::run(spec, dir);
            if let Some(p) = r.platform.as_mut().and_then(Value::as_object_mut) {
                p.insert("sha".into(), json!(sha));
            }
            r
        }
        Err(e) => Report {
            violations: vec![e.to_string()],
            ..report(spec, Outcome::Failed, &e.to_string())
        },
    }
}

/// The unscratch job: deletes a session's scratch ref when it ends; see
/// `.claude/decisions/2026-11-16-development-session-bench-containers.md` §4.
fn unscratch(spec: &JobSpec, dir: &Path) -> Report {
    let refname = format!("refs/heads/{}", spec.branch);
    if crate::core::bench::scratch_owner(&refname).is_none() {
        return report(
            spec,
            Outcome::Failed,
            "an unscratch job deletes only a session's scratch ref",
        );
    }
    let _ = std::fs::remove_dir_all(dir);
    match crate::checks::ci::delete_ref(dir, &spec.remote, &spec.token, &refname) {
        Ok(()) => report(spec, Outcome::Passed, &format!("deleted {refname}\n")),
        Err(e) => report(spec, Outcome::Failed, &e.to_string()),
    }
}

/// What a release job says when `main` moved past the deploy it releases.
pub const MAIN_MOVED: &str = "main moved; the next deploy releases";

/// The release job: the release bot on the deployed `main`, its output committed as
/// `chore(release): …` and pushed with a tag per release; see
/// `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §7. It builds no
/// image: image units' versions bump, and their images follow that ADR's step 5.
fn release(spec: &JobSpec, dir: &Path) -> Report {
    let failed = |why: String| Report {
        violations: vec![why.clone()],
        ..report(spec, Outcome::Failed, &why)
    };
    let _ = std::fs::remove_dir_all(dir);
    match crate::checks::ci::checkout_head(dir, &spec.remote, &spec.token, &spec.base, &spec.sha) {
        Ok(true) => {}
        Ok(false) => return report(spec, Outcome::Passed, MAIN_MOVED),
        Err(e) => return failed(e.to_string()),
    }
    let config_file = ".config/releases.yaml";
    let outcome = crate::release::Config::load(&dir.join(config_file))
        .and_then(|c| crate::release::run(dir, &c, config_file, "HEAD", None, false, false));
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => return failed(e.to_string()),
    };
    if outcome.written.is_empty() {
        return report(spec, Outcome::Passed, "nothing to release");
    }
    let (ok, log) = crate::checks::ci::exec(
        dir,
        &[
            "cargo".into(),
            "update".into(),
            "-w".into(),
            "--offline".into(),
        ],
        crate::checks::ci::TIMEOUT,
        &spec.token,
    );
    if !ok {
        return failed(format!("cargo update -w --offline failed\n{log}"));
    }
    let subject = release_subject(&outcome.released);
    let tags: Vec<String> = outcome.released.iter().map(|r| r.tag.clone()).collect();
    match crate::checks::ci::land(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &subject,
        spec.env
            .get(crate::control::COMMIT_AUTHOR)
            .map(String::as_str),
        &tags,
    ) {
        Ok(crate::checks::ci::Landed::Pushed(commit)) => {
            let mut log = format!("{subject} as {commit}\n");
            for t in &tags {
                let _ = writeln!(log, "tag {t}");
            }
            Report {
                merged: Some(commit),
                ..report(spec, Outcome::Passed, &log)
            }
        }
        Ok(crate::checks::ci::Landed::Moved) => report(spec, Outcome::Passed, MAIN_MOVED),
        Err(e) => failed(e.to_string()),
    }
}

/// `chore(release): <unit vX.Y.Z>[, …]`, units sorted; `chore(release): changelog` with none.
#[must_use]
pub fn release_subject(released: &[crate::release::Released]) -> String {
    let mut units: Vec<String> = released
        .iter()
        .map(|r| format!("{} v{}", r.unit, r.version))
        .collect();
    units.sort();
    if units.is_empty() {
        "chore(release): changelog".into()
    } else {
        format!("chore(release): {}", units.join(", "))
    }
}

/// The mirror job: `main` and the tags, fast-forward and add-only, to GitHub in a fresh bare
/// repository; on a change's branch, its head, force-pushed; the unmirror job deletes that
/// branch. See `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md` and
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §2.
fn mirror(spec: &JobSpec, dir: &Path) -> Report {
    let failed = |why: String| Report {
        violations: vec![why.clone()],
        ..report(spec, Outcome::Failed, &why)
    };
    let (Some(to), Some(to_token)) = (spec.env.get(MIRROR_TO), spec.env.get(MIRROR_TOKEN)) else {
        return failed("a mirror job names its mirror and its token".into());
    };
    let _ = std::fs::remove_dir_all(dir);
    let staged = dir.join("mirror.git");
    if let Err(e) = crate::git::bare(&staged) {
        return failed(e.to_string());
    }
    let scrub = |e: crate::infra::Error| failed(e.to_string().replace(to_token.as_str(), "***"));
    if spec.job == UNMIRROR {
        return match crate::git::delete_change(&spec.branch, to, Some(to_token), &staged) {
            Ok(()) => report(spec, Outcome::Passed, &format!("deleted {}\n", spec.branch)),
            Err(e) => scrub(e),
        };
    }
    if crate::core::change::branch(&spec.branch).is_some() {
        return match crate::git::push_change(
            &spec.remote,
            &spec.token,
            &spec.branch,
            &spec.sha,
            to,
            Some(to_token),
            &staged,
        ) {
            Ok(()) => report(
                spec,
                Outcome::Passed,
                &format!("{} {}\n", spec.sha, spec.branch),
            ),
            Err(e) => scrub(e),
        };
    }
    match crate::git::mirror_from(
        &spec.remote,
        &spec.token,
        &spec.branch,
        "mirror",
        to,
        Some(to_token),
        &staged,
    ) {
        Ok(done) => {
            let mut log = String::new();
            for (r, sha) in &done.refs {
                let _ = writeln!(log, "{sha} {r}");
            }
            report(spec, Outcome::Passed, &log)
        }
        Err(e) => scrub(e),
    }
}

/// The deploy job: the instance's sites whose inputs changed since the last deploy, built and
/// published with the job's credentials, then the Workers whose inputs changed, then `pigeon`
/// for job Containers; see
/// `.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md`.
/// A deploy group's files changed since its own baseline, none for everything, logged.
fn group_files(spec: &JobSpec, dir: &Path, group: &str, log: &mut String) -> Option<Vec<String>> {
    let files = crate::control::group_since(spec, group)
        .and_then(|since| crate::checks::ci::changed_since(dir, &since, &spec.sha).ok());
    let _ = match &files {
        Some(f) => writeln!(
            log,
            "{group}: {} file(s) changed since its baseline",
            f.len()
        ),
        None => writeln!(log, "{group}: everything"),
    };
    files
}

/// The deploy log's first line: the `pigeon` running it and the build `pigeon-start` was handed.
fn runs_as(exe: &str, handed: Option<&str>) -> String {
    format!("pigeon: {exe} (build {})\n", handed.unwrap_or("none"))
}

/// Why the deploy mustn't go on: it runs `exe`, handed build `handed` (or none), when main's
/// stored build is `want`; `None` when they match or no build is named. See the blue-green ADR's
/// 2026-11-18 rule that a deploy ships on the build before its own.
fn stale_build(exe: &str, handed: Option<&str>, want: Option<&str>) -> Option<String> {
    let want = want.filter(|w| !w.is_empty())?;
    if handed == Some(want) {
        return None;
    }
    Some(format!(
        "the deploy runs {exe} (build {}), not main's stored build {want}: the job's Container \
         didn't get or fetch it, so this deploy stops before any step; deploy again",
        handed.unwrap_or("none")
    ))
}

#[expect(clippy::too_many_lines, reason = "one step per deploy group")]
fn deploy(spec: &JobSpec, dir: &Path) -> Report {
    let failed = |why: String, log: &str| Report {
        violations: vec![why],
        ..report(spec, Outcome::Failed, log)
    };
    if let Err(e) = crate::checks::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return failed(e.to_string(), "");
    }
    let files = match &spec.since {
        Some(since) => crate::checks::ci::changed_since(dir, since, &spec.sha).ok(),
        None => None,
    };
    let exe =
        std::env::current_exe().map_or_else(|_| "unknown".into(), |p| p.display().to_string());
    // `pigeon-start`'s build checksum, `containers::worker::BUILD_SHA256_VAR`.
    let handed = std::env::var("PIGEON_BUILD_SHA256").ok();
    let mut log = runs_as(&exe, handed.as_deref());
    let want = spec
        .env
        .get(crate::wire::ship_wire::BUILD_VAR)
        .map(String::as_str);
    if let Some(why) = stale_build(&exe, handed.as_deref(), want) {
        return failed(why, &log);
    }
    match &files {
        Some(f) => {
            let _ = writeln!(log, "{} file(s) changed since the last deploy", f.len());
        }
        None => log.push_str("first deploy, or the last one is unknown: everything\n"),
    }
    // Every value is scrubbed but the stored build's SHA-256, which the log names on purpose.
    let secrets: Vec<String> = spec
        .env
        .iter()
        .filter(|(k, _)| k.as_str() != crate::wire::ship_wire::BUILD_VAR)
        .map(|(_, v)| v.clone())
        .collect();
    let scrub = |t: &str| {
        secrets
            .iter()
            .filter(|s| s.len() >= 8)
            .fold(t.to_string(), |acc, s| acc.replace(s.as_str(), "…"))
    };
    let site_files = group_files(spec, dir, "sites", &mut log);
    let worker_files = group_files(spec, dir, "workers", &mut log);
    let build_files = group_files(spec, dir, "build:pigeon", &mut log);
    let judge_files = group_files(spec, dir, "build:judge", &mut log);
    let mut passed: Vec<String> = Vec::new();
    let ios = crate::iosrelease::triggers(files.as_deref());
    let mut violations = Vec::new();
    let published = match publish_sites(spec, dir, site_files.as_deref(), &mut log)
        .inspect(|_| passed.push("sites".to_string()))
    {
        Ok(p) => p,
        Err(why) => {
            violations.push(why);
            Vec::new()
        }
    };
    // Main's head is checked again before each costly step; see the deploys ADR's 2026-11-16
    // amendment.
    let superseded = |images: Option<crate::control::Images>, log: &str| {
        crate::ship::superseded(spec, dir).map(|why| Report {
            published: published.clone(),
            images,
            violations: vec![why.clone()],
            ..report(spec, Outcome::Superseded, &scrub(&format!("{log}{why}\n")))
        })
    };
    if let Some(r) = superseded(None, &log) {
        return r;
    }
    let mut images = None;
    let shipped = crate::ship::workers(spec, dir, worker_files.as_deref(), &mut log, &mut images)
        .inspect(|_| passed.push("workers".to_string()));
    // Only a Workers step that shipped nothing, and didn't fail, gives way to a newer push; a failure
    // is reported as one.
    let quiet = matches!(&shipped, Ok(w) if w.is_empty());
    let workers = match shipped {
        Ok(w) => w,
        Err(why) => {
            violations.push(why);
            Vec::new()
        }
    };
    if quiet && let Some(r) = superseded(images.clone(), &log) {
        return r;
    }
    let build = match crate::ship::build(
        spec,
        dir,
        build_files.as_deref(),
        judge_files.as_deref(),
        &mut log,
    )
    .inspect(|_| passed.extend(["build:pigeon".to_string(), "build:judge".to_string()]))
    {
        Ok(b) => b,
        Err(why) => {
            violations.push(why);
            None
        }
    };
    let ledger = match crate::infra::catalog::ledger_resolved(dir, dir, &spec.sha) {
        Ok(l) => l,
        Err(problems) => {
            let _ = writeln!(log, "grants ledger not kept: {}", problems.join("; "));
            None
        }
    };
    let pipeline = files
        .as_deref()
        .is_none_or(crate::core::deploy::serving::touches_pipeline);
    let serving = crate::ship::serving(spec, dir, &mut log);
    let violations: Vec<String> = violations.iter().map(|v| scrub(v)).collect();
    for v in &violations {
        let _ = writeln!(log, "violation: {v}");
    }
    Report {
        published,
        workers,
        passed,
        ios,
        build,
        images,
        ledger,
        serving,
        pipeline,
        violations: violations.clone(),
        ..report(
            spec,
            if violations.is_empty() {
                Outcome::Passed
            } else {
                Outcome::Failed
            },
            &scrub(&log),
        )
    }
}

/// The instance's sites whose inputs changed among `files`, published; their units.
fn publish_sites(
    spec: &JobSpec,
    dir: &Path,
    files: Option<&[String]>,
    log: &mut String,
) -> Result<Vec<String>, String> {
    let sites = crate::checks::scope::deploys(dir, files).map_err(|e| e.to_string())?;
    if sites.is_empty() {
        log.push_str("no site to publish\n");
        return Ok(Vec::new());
    }
    let instance = Instance::load(dir).map_err(|e| e.to_string())?;
    let mut args = Map::new();
    args.insert("sites".into(), json!(sites));
    args.insert("deployed".into(), json!(spec.sha));
    let request = Request {
        operation: "sites.publish".into(),
        args,
    };
    // The architecture data names the policies, through the Container's pigeon-eviction.
    let var = |k: &str| {
        spec.env.get(k).cloned().or_else(|| {
            (k == crate::eviction::VAR)
                .then(|| std::env::var(k).ok())
                .flatten()
        })
    };
    let r = run_with(&instance, &request, var).map_err(|e| e.to_string())?;
    let _ = writeln!(log, "{}", r.result);
    if r.ok {
        Ok(sites)
    } else {
        Err(r.violations.join("; "))
    }
}

/// The merge job: the approved head's tree, squashed onto the base as `<title> (!<number>)`.
fn merge(spec: &JobSpec, dir: &Path) -> Report {
    let mut message = format!(
        "{} (!{})\n\n{}\n\n",
        spec.title,
        spec.number,
        spec.plan.trim()
    );
    if let Some(by) = &spec.approver {
        let _ = writeln!(
            message,
            "Change-Approved-By: {}",
            crate::control::public_approver(by)
        );
    }
    let _ = writeln!(message, "Change-Head: {}", spec.sha);
    match crate::checks::ci::squash_onto(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
        &message,
        spec.env
            .get(crate::control::COMMIT_AUTHOR)
            .map(String::as_str),
    ) {
        Ok(commit) => Report {
            log: format!("merged {} as {commit}\n", spec.sha),
            merged: Some(commit),
            ..report(spec, Outcome::Passed, "")
        },
        Err(e) => Report {
            violations: vec![e.to_string()],
            ..report(spec, Outcome::Failed, &e.to_string())
        },
    }
}

/// ADRs only gain amendments, so a restack merges two appends to one by union.
const UNION: &str = ".claude/decisions/*.md";

/// The restack job: the head replayed onto the base as it now stands, and pushed to the
/// change's branch when git merges it; see
/// `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.
fn restack(spec: &JobSpec, dir: &Path) -> Report {
    match crate::checks::ci::restack(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
        spec.env.get(crate::control::AFTER_HEAD).map(String::as_str),
        &[UNION],
    ) {
        Ok(r) => {
            let relation = match r.relation {
                crate::checks::ci::Relation::Current => Relation::Current,
                crate::checks::ci::Relation::Disjoint => Relation::Disjoint,
                crate::checks::ci::Relation::Clean => Relation::Clean,
                crate::checks::ci::Relation::Conflicting => Relation::Conflicting,
            };
            let mut log = format!(
                "{} onto {}: {relation:?}{}\n",
                spec.sha,
                r.onto,
                r.to.as_deref()
                    .map(|t| format!(" as {t}"))
                    .unwrap_or_default()
            );
            let carries = (r.to.is_some() && r.same_patch)
                .then(|| carried(spec, dir, &r.main_files, &r.head_files, &mut log));
            Report {
                log,
                restack: Some(Restacked {
                    relation,
                    to: r.to,
                    same_patch: r.same_patch,
                    conflicts: r.conflicts,
                    onto: r.onto,
                    context: r
                        .context
                        .into_iter()
                        .map(|c| ConflictContext {
                            path: c.path,
                            commits: c.commits,
                            base_diff: c.base_diff,
                            change_diff: c.change_diff,
                        })
                        .collect(),
                    messages: r.messages,
                    landed: r.landed,
                    carries,
                }),
                ..report(spec, Outcome::Passed, "")
            }
        }
        Err(e) => Report {
            violations: vec![e.to_string()],
            ..report(spec, Outcome::Failed, &e.to_string())
        },
    }
}

/// The head's passed checks and gate jobs, from [`crate::control::KEPT`], that carry to the
/// restacked head touching `head`, checked out in `dir`, over `main`'s changed `files`; none
/// while `main`'s test map is red. See the merge-path ADR's 2026-11-16 amendment (#20).
fn carried(
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    head: &[String],
    log: &mut String,
) -> Vec<String> {
    let kept: Vec<Check> = spec
        .env
        .get(crate::control::KEPT)
        .and_then(|k| serde_json::from_str(k).ok())
        .unwrap_or_default();
    let map = match std::env::var(crate::wire::ship_wire::TEST_MAP_RED_VAR) {
        Ok(sha) if !sha.is_empty() => {
            let _ = writeln!(log, "test map: main's failed at {sha}; nothing carries");
            None
        }
        _ => test_map(log),
    };
    match crate::checks::scope::carried(dir, files, head, &kept, map.as_ref()) {
        Ok(carried) => {
            for name in &carried {
                let _ = writeln!(log, "carries: {name}");
            }
            carried.into_iter().collect()
        }
        Err(e) => {
            let _ = writeln!(log, "reach: {e}; nothing carries");
            Vec::new()
        }
    }
}

/// The bound job: the change and pull-request policies, with the change's approved scope and
/// its plan as the body, then the checks its paths need.
fn bound(spec: &JobSpec, dir: &Path, judge: &Path) -> Report {
    let (since, built) = built_on(spec, dir);
    let files = match crate::checks::ci::changed(dir, since, &spec.sha) {
        Ok(f) => f,
        Err(e) => {
            let mut r = report(spec, Outcome::Failed, &e.to_string());
            r.violations = vec![e.to_string()];
            return r;
        }
    };
    let exe =
        std::env::current_exe().map_or_else(|_| "unknown".into(), |p| p.display().to_string());
    let mut log = format!(
        "pigeon: {exe}\n{built}{} file(s) changed:\n{}\n",
        files.len(),
        files.join("\n")
    );
    let rules = base_rules(spec, dir);
    let tiers = rules.as_ref().map(|r| tiers(spec, dir, &files, r));
    if tiers.is_none() {
        log.push_str("tiers: none, as main has no readable rules\n");
    }
    if let Some(t) = &tiers {
        let _ = writeln!(
            log,
            "tiers: authorize {:?} ({}), merge {:?} ({}), close {:?} ({})",
            t.authorize.tier,
            t.authorize.rule,
            t.merge.tier,
            t.merge.rule,
            t.close.tier,
            t.close.rule
        );
    }
    let mut violations = match policy(spec, dir, &files, judge, &mut log) {
        Ok(v) => v,
        Err(e) => vec![e],
    };
    violations.extend(
        rules
            .as_ref()
            .and_then(|r| crate::core::approval::tiers::mixed_with_eviction(r, &files)),
    );
    for v in &violations {
        let _ = writeln!(log, "violation: {v}");
    }
    let map = violations.is_empty().then(|| test_map(&mut log)).flatten();
    let checks = if violations.is_empty() {
        match crate::checks::scope::jobs_with(dir, &files, map.as_ref()) {
            Ok(jobs) => jobs
                .into_iter()
                .filter(|j| j.name != "policy.check")
                .map(|j| Check {
                    name: j.name,
                    argv: j.argv,
                })
                .collect(),
            Err(e) => {
                let mut r = report(spec, Outcome::Failed, &log);
                r.violations = vec![e.to_string()];
                return r;
            }
        }
    } else {
        Vec::new()
    };
    for c in &checks {
        let _ = writeln!(log, "check: {}", c.name);
    }
    let full = full_set(&checks, &mut log);
    let gate = (!full.is_empty()).then(|| gate(dir, &files, map.as_ref(), &mut log));
    let predicted = forecast(spec, dir, &mut log);
    let seen = (since != spec.base)
        .then(|| crate::checks::ci::built_on_head(dir, since, &spec.sha).ok())
        .flatten();
    let outcome = if violations.is_empty() {
        Outcome::Passed
    } else {
        Outcome::Failed
    };
    Report {
        violations,
        checks,
        full,
        gate,
        patch: crate::checks::ci::head_patch(dir, since, &spec.sha),
        own: crate::checks::ci::own_digest(dir, since, &spec.sha),
        tiers,
        forecast: predicted,
        built_on: seen,
        ..Report::of(&spec.id, outcome, &log)
    }
}

/// `main`'s test map, from `PIGEON_TEST_MAP`, which `pigeon-start` exports; none without one, and
/// the tests are then chosen by crate. See the 2026-11-11 amendment to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
fn test_map(log: &mut String) -> Option<crate::checks::scope::TestMap> {
    let Ok(path) = std::env::var(TEST_MAP_VAR) else {
        log.push_str("test map: none; tests by crate\n");
        return None;
    };
    match crate::checks::scope::TestMap::read(std::path::Path::new(&path)) {
        Ok(m) => {
            let _ = writeln!(log, "test map: {} binaries, of {}", m.binaries.len(), m.sha);
            Some(m)
        }
        Err(e) => {
            let _ = writeln!(log, "test map: {e}; tests by crate");
            None
        }
    }
}

/// How much of the full set the head's merge gate runs, logged; full when the workspace can't
/// be read. See the 2026-11-12 amendment on the merge gate to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
fn gate(
    dir: &Path,
    files: &[String],
    map: Option<&crate::checks::scope::TestMap>,
    log: &mut String,
) -> crate::core::deploy::gate::Gate {
    use crate::core::deploy::gate::Gate;
    let gate = crate::checks::scope::gate(dir, files, map)
        .unwrap_or_else(|e| Gate::Full(format!("the workspace isn't readable ({e})")));
    match &gate {
        Gate::Scoped => log.push_str("merge gate: scoped; the test map chose the tests\n"),
        Gate::Full(why) => {
            let _ = writeln!(log, "merge gate: full; {why}");
        }
    }
    gate
}

/// The variable `pigeon-start` exports the test map's path in.
pub const TEST_MAP_VAR: &str = "PIGEON_TEST_MAP";

/// The head's merge gate set: its own code checks, and the whole test suite, which only a full
/// gate runs; none for a head with no Rust input. See
/// `.claude/decisions/2026-11-15-development-code-checks-by-reach.md` §4.
fn full_set(checks: &[Check], log: &mut String) -> Vec<Check> {
    let mut full: Vec<Check> = checks
        .iter()
        .filter(|c| crate::actions::on_github(&c.argv))
        .cloned()
        .collect();
    if full.is_empty() {
        return full;
    }
    let suite = ["cargo", "test", "--workspace", "--locked"].map(String::from);
    if !full.iter().any(|c| c.argv == suite) {
        full.push(Check {
            name: "test".into(),
            argv: suite.to_vec(),
        });
    }
    let _ = writeln!(log, "merge gate: {} check(s)", full.len());
    full
}

/// The head's approval tiers, by the rules on the base branch; none when the base has none; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §2.
fn tiers(
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    rules: &crate::core::approval::tiers::Rules,
) -> crate::core::approval::tiers::Tiers {
    let decisions = rules.decisions.clone().unwrap_or_default();
    let appends_only = !decisions.is_empty()
        && files
            .iter()
            .filter(|f| f.starts_with(&decisions))
            .all(|f| crate::checks::ci::appends_only(dir, &spec.base, &spec.sha, f));
    let adds_secret = files
        .iter()
        .filter(|f| {
            rules
                .secret_lists
                .iter()
                .any(|g| crate::core::approval::tiers::matches(g, f))
        })
        .any(|f| adds_secret(dir, &spec.base, f));
    let on = |globs: &[String], f: &str| {
        globs
            .iter()
            .any(|g| crate::core::approval::tiers::matches(g, f))
    };
    let secret_material = files.iter().filter(|f| on(&rules.secret_code, f)).any(|f| {
        crate::checks::ci::secret_material(dir, &spec.base, &spec.sha, f, &rules.secret_markers)
    });
    let gate = rules.gate.clone().unwrap_or_default();
    let gated: Vec<&String> = files.iter().filter(|f| on(&gate, f)).collect();
    let comments_only = !gated.is_empty()
        && gated
            .iter()
            .all(|f| crate::checks::ci::comments_only(dir, &spec.base, &spec.sha, f));
    let caps = crate::core::approval::tiers::CAPABILITIES;
    let renames_only = files.iter().any(|f| f == caps)
        && crate::checks::ci::renames_only(dir, &spec.base, &spec.sha, caps);
    let facts = crate::core::approval::tiers::Facts {
        secret_material,
        comments_only,
        renames_only,
        deploys_worker: crate::checks::scope::workers(dir, Some(files))
            .map_or(true, |w| !w.is_empty()),
        appends_only,
        adds_secret,
    };
    crate::core::approval::tiers::classify(rules, &spec.scope, files, facts)
}

/// The approval rules on the base branch; none when it has none, or they don't read.
fn base_rules(spec: &JobSpec, dir: &Path) -> Option<crate::core::approval::tiers::Rules> {
    let text = crate::checks::ci::base_file(dir, &spec.base, crate::core::approval::tiers::RULES)?;
    serde_saphyr::from_str(&text).ok()
}

/// Whether the head's `secrets:` list in `path` names one its base's doesn't; an unreadable
/// head counts as adding one, so the owner is asked.
fn adds_secret(dir: &Path, base: &str, path: &str) -> bool {
    let names = |text: &str| -> Option<Vec<String>> {
        let v: serde_json::Value = serde_saphyr::from_str(text).ok()?;
        Some(
            v["secrets"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        )
    };
    let Some(head) = std::fs::read_to_string(dir.join(path))
        .ok()
        .and_then(|t| names(&t))
    else {
        return true;
    };
    let before = crate::checks::ci::base_file(dir, base, path)
        .and_then(|t| names(&t))
        .unwrap_or_default();
    crate::core::approval::tiers::lengthens(&before, &head)
}

/// The head's violations of the base's policies, judged by `judge`, `main`'s
/// `pigeon-eviction`; a judge that can't answer is a violation, never a pass; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §2.
fn policy(
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    judge: &Path,
    log: &mut String,
) -> Result<Vec<String>, String> {
    let sum = std::env::var(crate::wire::ship_wire::JUDGE_SHA256_VAR)
        .ok()
        .unwrap_or_default();
    let _ = writeln!(log, "eviction: {} (sha256 {sum})", judge.display());
    match base_policies(spec, dir)? {
        Some(p) => judge_with(judge, spec, dir, files, &p),
        None => Ok(Vec::new()),
    }
}

/// The violations `binary`, a `pigeon-eviction`, finds in the head at `dir` against `policies`,
/// with the change as its pull request.
///
/// # Errors
///
/// When it didn't run, couldn't judge (an invalid policy), or printed no JSON report.
pub fn judge_with(
    binary: &Path,
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    policies: &Path,
) -> Result<Vec<String>, String> {
    let counts = crate::checks::ci::rust_diff(dir, &spec.base, &spec.sha)
        .map(|d| crate::core::testguard::count(&d))
        .unwrap_or_default();
    let reason = crate::checks::ci::first_trailer(dir, &spec.base, &spec.sha, "Test-Reason")
        .unwrap_or_default();
    let pr = [
        ("PR_TITLE", spec.title.clone()),
        ("PR_BODY", spec.plan.clone()),
        ("PR_NUMBER", spec.number.to_string()),
        ("PR_FILES", files.join("\n")),
        ("PR_BRANCH", spec.branch.clone()),
        ("PR_SCOPE", spec.scope.join("\n")),
        ("PR_ADDED_TESTS", counts.added_tests.to_string()),
        ("PR_ENUMERATIONS", counts.enumerations.to_string()),
        ("PR_TEST_REASON", reason),
    ];
    crate::eviction::check(binary, dir, policies, Some(&pr))
        .map(|r| r.violations)
        .map_err(|e| e.to_string())
}

/// The eviction job: each candidate in [`crate::control::CANDIDATES`] judged by `judge`
/// against `main`'s policies; a judge that can't answer fails the job, so nothing is granted;
/// see `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4-§5.
fn eviction(spec: &JobSpec, dir: &Path, judge: &Path) -> Report {
    let failed = |why: String| Report {
        violations: vec![why.clone()],
        ..report(spec, Outcome::Failed, &why)
    };
    let Some(text) = spec.env.get(crate::control::CANDIDATES) else {
        return failed("an eviction job names its candidates".into());
    };
    let candidates: Vec<Value> = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => return failed(format!("its candidates aren't a JSON array: {e}")),
    };
    let policies = match base_policies(spec, dir) {
        Ok(Some(p)) => p,
        Ok(None) => return report(spec, Outcome::Passed, "main names no policies\n"),
        Err(e) => return failed(e),
    };
    let mut log = String::new();
    let mut verdicts = Vec::new();
    for c in &candidates {
        let number = c["number"].clone();
        match crate::eviction::evict(judge, dir, &policies, &c["fields"]) {
            Ok(v) => {
                let _ = writeln!(log, "!{number}: {}", v["granted"]);
                verdicts.push(serde_json::json!({
                    "number": number,
                    "granted": v["granted"],
                    "results": v["results"],
                }));
            }
            Err(e) => return failed(e.to_string()),
        }
    }
    Report {
        evictions: verdicts,
        ..report(spec, Outcome::Passed, &log)
    }
}

/// The branch the head's files are counted from: [`crate::control::AFTER`]'s, fetched, for a
/// change built on another, else the base; and a line saying which; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.
fn built_on<'a>(spec: &'a JobSpec, dir: &Path) -> (&'a str, String) {
    let Some(after) = spec.env.get(crate::control::AFTER) else {
        return (&spec.base, String::new());
    };
    match crate::checks::ci::fetch_branch(dir, &spec.remote, &spec.token, after) {
        Ok(()) => (after, format!("built on {after}\n")),
        Err(e) => (
            &spec.base,
            format!(
                "built on {after}, which didn't fetch ({e}); counted from {}\n",
                spec.base
            ),
        ),
    }
}

/// The open changes in [`crate::control::OVERLAPS`] the head would conflict with, each with its
/// paths; never a violation; see the 2026-11-12 ADR §5's conflict forecasts.
fn forecast(spec: &JobSpec, dir: &Path, log: &mut String) -> Vec<Value> {
    let others: Vec<(u32, String)> = spec
        .env
        .get(crate::control::OVERLAPS)
        .and_then(|t| serde_json::from_str::<Vec<Value>>(t).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|o| {
            let n = u32::try_from(o["number"].as_u64()?).ok()?;
            Some((n, o["branch"].as_str()?.to_string()))
        })
        .collect();
    crate::checks::ci::forecast(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.sha,
        &others,
        &[UNION],
    )
    .into_iter()
    .map(|(n, paths)| {
        let _ = writeln!(log, "forecast: conflicts with !{n} in {}", paths.join(", "));
        serde_json::json!({ "number": n, "paths": paths })
    })
    .collect()
}

/// The base's policy directory, as the base's manifest names it, written under the checkout's
/// `.git`; none when the base names none. A change is never judged by rules it edits; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §2.
fn base_policies(spec: &JobSpec, dir: &Path) -> Result<Option<PathBuf>, String> {
    let manifest = crate::instance::MANIFEST;
    let Some(text) = crate::checks::ci::base_file(dir, &spec.base, manifest) else {
        return Ok(None);
    };
    let v: Value =
        serde_saphyr::from_str(&text).map_err(|e| format!("{}'s {manifest}: {e}", spec.base))?;
    let Some(rel) = v["paths"]["policies"].as_str() else {
        return Ok(None);
    };
    let out = dir.join(".git").join("pigeon-base-policies");
    let _ = std::fs::remove_dir_all(&out);
    crate::checks::ci::base_tree(dir, &spec.base, rel, &out).map_err(|e| e.to_string())?;
    Ok(Some(out))
}

/// A job request's answer: its report so far, or why it was refused.
#[must_use]
pub fn submit(board: &Arc<Board>, body: &[u8]) -> (u16, Value) {
    match serde_json::from_slice::<JobSpec>(body) {
        Ok(spec)
            if spec
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !spec.id.is_empty() =>
        {
            (202, json!(board.start(spec)))
        }
        Ok(_) => (
            400,
            json!({ "error": "a job ID is letters, digits and `-`" }),
        ),
        Err(e) => (400, json!({ "error": format!("not a job: {e}") })),
    }
}

/// Job `id`'s report, or 404.
#[must_use]
pub fn status(board: &Board, id: &str) -> (u16, Value) {
    board.get(id).map_or_else(
        || (404, json!({ "error": "no such job" })),
        |mut r| {
            // A running pool job's log so far; see the pool ADR's 2026-11-17 log entry.
            if r.outcome == Outcome::Running
                && let Ok(so_far) = std::fs::read_to_string(crate::pool_job::live_log(id))
            {
                r.log = tail(&so_far).to_string();
            }
            (200, json!(r))
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_deploy_stops_unless_it_runs_the_stored_build() {
        let run = "/tmp/pigeon-build";
        assert_eq!(super::stale_build(run, Some("ab"), Some("ab")), None);
        assert_eq!(super::stale_build(run, None, None), None, "no build named");
        assert_eq!(super::stale_build(run, Some("ab"), Some("")), None);
        let why = super::stale_build("/usr/local/bin/pigeon", None, Some("ab")).expect("stale");
        assert!(why.contains("/usr/local/bin/pigeon (build none)"), "{why}");
        assert!(why.contains("stored build ab"), "{why}");
        assert!(super::stale_build(run, Some("cd"), Some("ab")).is_some());
    }

    #[test]
    fn the_deploy_log_names_its_pigeon_and_the_build_it_was_handed() {
        assert_eq!(
            super::runs_as("/tmp/pigeon-build", Some("ab12")),
            "pigeon: /tmp/pigeon-build (build ab12)\n"
        );
        assert_eq!(
            super::runs_as("/usr/local/bin/pigeon", None),
            "pigeon: /usr/local/bin/pigeon (build none)\n"
        );
    }
}
