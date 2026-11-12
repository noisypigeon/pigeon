//! A job Container's work: `pigeon backplane jobs` takes a [`JobSpec`] over the signed link,
//! checks out its head, runs it in the background, and reports; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §3, §4.
//!
//! The bound job is sentinel's change policy and `change.scope` on the checked-out tree, with
//! the change's approved scope; every other job is a command `change.scope` named.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use crate::changes::{BOUND, Check, ConflictContext, MERGE, RESTACK, Relation, Restacked};
use crate::control::{
    DEPLOY, JobSpec, MIRROR, MIRROR_TO, MIRROR_TOKEN, Outcome, Report, UNMIRROR, tail,
};
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
            merged: None,
            published: Vec::new(),
            workers: Vec::new(),
            build: None,
            images: None,
            tiers: None,
            restack: None,
            platform: None,
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
        merged: None,
        published: Vec::new(),
        workers: Vec::new(),
        build: None,
        images: None,
        tiers: None,
        restack: None,
        platform: None,
    }
}

/// Runs `spec` in `dir`: checks out its head, then the bound job or its command; the bound job
/// judges with the `pigeon-sentinel` [`crate::sentinel::locate`] finds.
#[must_use]
pub fn run(spec: &JobSpec, dir: &Path) -> Report {
    run_judged(
        spec,
        dir,
        &crate::sentinel::locate(&|k| std::env::var(k).ok()),
    )
}

/// [`run`], the bound job judging with `sentinel`.
#[must_use]
pub fn run_judged(spec: &JobSpec, dir: &Path, sentinel: &Path) -> Report {
    // A merge whose report was lost may already be on the base; finding it there is the merge.
    if spec.job == MERGE
        && let Ok(Some(commit)) =
            crate::cobot::ci::merged_already(dir, &spec.remote, &spec.token, &spec.base, &spec.sha)
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
    let _ = std::fs::remove_dir_all(dir);
    if let Err(e) = crate::cobot::ci::checkout(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
    ) {
        let mut r = report(spec, Outcome::Failed, &e.to_string());
        if spec.job == BOUND {
            r.violations = vec![e.to_string()];
        }
        return r;
    }
    if spec.job == BOUND {
        return bound(spec, dir, sentinel);
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
    if let Err(e) = crate::cobot::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return report(spec, Outcome::Failed, &e.to_string());
    }
    let workspace = dir.join(crate::cobot::scope::WORKSPACE);
    let (ok, log) = crate::cobot::ci::exec(
        &workspace,
        &spec.argv,
        crate::cobot::ci::TIMEOUT,
        &spec.token,
    );
    report(
        spec,
        if ok { Outcome::Passed } else { Outcome::Failed },
        &log,
    )
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
    if let Err(e) = crate::cobot::git::bare(&staged) {
        return failed(e.to_string());
    }
    let scrub = |e: crate::cobot::Error| failed(e.to_string().replace(to_token.as_str(), "***"));
    if spec.job == UNMIRROR {
        return match crate::cobot::git::delete_change(&spec.branch, to, Some(to_token), &staged) {
            Ok(()) => report(spec, Outcome::Passed, &format!("deleted {}\n", spec.branch)),
            Err(e) => scrub(e),
        };
    }
    if crate::changes::branch(&spec.branch).is_some() {
        return match crate::cobot::git::push_change(
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
    match crate::cobot::git::mirror_from(
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
fn deploy(spec: &JobSpec, dir: &Path) -> Report {
    let failed = |why: String, log: &str| Report {
        violations: vec![why],
        ..report(spec, Outcome::Failed, log)
    };
    if let Err(e) = crate::cobot::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return failed(e.to_string(), "");
    }
    let files = match &spec.since {
        Some(since) => crate::cobot::ci::changed_since(dir, since, &spec.sha).ok(),
        None => None,
    };
    let mut log = match &files {
        Some(f) => format!("{} file(s) changed since the last deploy\n", f.len()),
        None => "first deploy, or the last one is unknown: everything\n".to_string(),
    };
    let secrets: Vec<String> = spec.env.values().cloned().collect();
    let scrub = |t: &str| {
        secrets
            .iter()
            .filter(|s| s.len() >= 8)
            .fold(t.to_string(), |acc, s| acc.replace(s.as_str(), "…"))
    };
    let mut violations = Vec::new();
    let published = match publish_sites(spec, dir, files.as_deref(), &mut log) {
        Ok(p) => p,
        Err(why) => {
            violations.push(why);
            Vec::new()
        }
    };
    let mut images = None;
    let workers = match crate::ship::workers(spec, dir, files.as_deref(), &mut log, &mut images) {
        Ok(w) => w,
        Err(why) => {
            violations.push(why);
            Vec::new()
        }
    };
    let build = match crate::ship::build(spec, dir, files.as_deref(), &mut log) {
        Ok(b) => b,
        Err(why) => {
            violations.push(why);
            None
        }
    };
    let violations: Vec<String> = violations.iter().map(|v| scrub(v)).collect();
    for v in &violations {
        let _ = writeln!(log, "violation: {v}");
    }
    Report {
        published,
        workers,
        build,
        images,
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
    let sites = crate::cobot::scope::deploys(dir, files).map_err(|e| e.to_string())?;
    if sites.is_empty() {
        log.push_str("no site to publish\n");
        return Ok(Vec::new());
    }
    let instance = Instance::load(dir).map_err(|e| e.to_string())?;
    let mut args = Map::new();
    args.insert("sites".into(), json!(sites));
    let request = Request {
        operation: "sites.publish".into(),
        args,
    };
    // The architecture data names the policies, through the Container's pigeon-sentinel.
    let var = |k: &str| {
        spec.env.get(k).cloned().or_else(|| {
            (k == crate::sentinel::VAR)
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
    match crate::cobot::ci::squash_onto(
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
    match crate::cobot::ci::restack(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
        &[UNION],
    ) {
        Ok(r) => {
            let relation = match r.relation {
                crate::cobot::ci::Relation::Current => Relation::Current,
                crate::cobot::ci::Relation::Disjoint => Relation::Disjoint,
                crate::cobot::ci::Relation::Clean => Relation::Clean,
                crate::cobot::ci::Relation::Conflicting => Relation::Conflicting,
            };
            Report {
                log: format!(
                    "{} onto {}: {relation:?}{}\n",
                    spec.sha,
                    r.onto,
                    r.to.as_deref()
                        .map(|t| format!(" as {t}"))
                        .unwrap_or_default()
                ),
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

/// The bound job: the change and pull-request policies, with the change's approved scope and
/// its plan as the body, then the checks its paths need.
fn bound(spec: &JobSpec, dir: &Path, sentinel: &Path) -> Report {
    let files = match crate::cobot::ci::changed(dir, &spec.base, &spec.sha) {
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
        "pigeon: {exe}\n{} file(s) changed:\n{}\n",
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
    let mut violations = match policy(spec, dir, &files, sentinel, &mut log) {
        Ok(v) => v,
        Err(e) => vec![e],
    };
    violations.extend(
        rules
            .as_ref()
            .and_then(|r| crate::approvals::mixed_with_sentinel(r, &files)),
    );
    for v in &violations {
        let _ = writeln!(log, "violation: {v}");
    }
    let map = violations.is_empty().then(|| test_map(&mut log)).flatten();
    let checks = if violations.is_empty() {
        match crate::cobot::scope::jobs_with(dir, &files, map.as_ref()) {
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
    let full = full_set(dir, &checks, &mut log);
    let gate = (!full.is_empty()).then(|| gate(dir, &files, map.as_ref(), &mut log));
    Report {
        id: spec.id.clone(),
        outcome: if violations.is_empty() {
            Outcome::Passed
        } else {
            Outcome::Failed
        },
        log: tail(&log).to_string(),
        violations,
        checks,
        full,
        gate,
        merged: None,
        published: Vec::new(),
        workers: Vec::new(),
        build: None,
        images: None,
        tiers,
        restack: None,
        platform: None,
    }
}

/// `main`'s test map, from `PIGEON_TEST_MAP`, which `pigeon-start` exports; none without one, and
/// the tests are then chosen by crate. See the 2026-11-11 amendment to
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.
fn test_map(log: &mut String) -> Option<crate::cobot::scope::TestMap> {
    let Ok(path) = std::env::var(TEST_MAP_VAR) else {
        log.push_str("test map: none; tests by crate\n");
        return None;
    };
    match crate::cobot::scope::TestMap::read(std::path::Path::new(&path)) {
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
    map: Option<&crate::cobot::scope::TestMap>,
    log: &mut String,
) -> crate::core::deploy::gate::Gate {
    use crate::core::deploy::gate::Gate;
    let gate = crate::cobot::scope::gate(dir, files, map)
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

/// The head's full set of code checks, which the merge gate runs, when `checks` holds any code
/// check; none for a head with no Rust input. See
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §5.
fn full_set(dir: &Path, checks: &[Check], log: &mut String) -> Vec<Check> {
    if !checks.iter().any(|c| crate::actions::on_github(&c.argv)) {
        return Vec::new();
    }
    match crate::cobot::scope::full(dir) {
        Ok(jobs) => {
            let full: Vec<Check> = jobs
                .into_iter()
                .filter(|j| crate::actions::on_github(&j.argv))
                .map(|j| Check {
                    name: j.name,
                    argv: j.argv,
                })
                .collect();
            let _ = writeln!(log, "merge gate: {} check(s)", full.len());
            full
        }
        Err(e) => {
            let _ = writeln!(log, "merge gate: the full set isn't readable ({e})");
            Vec::new()
        }
    }
}

/// The head's approval tiers, by the rules on the base branch; none when the base has none; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §2.
fn tiers(
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    rules: &crate::approvals::Rules,
) -> crate::approvals::Tiers {
    let decisions = rules.decisions.clone().unwrap_or_default();
    let appends_only = !decisions.is_empty()
        && files
            .iter()
            .filter(|f| f.starts_with(&decisions))
            .all(|f| crate::cobot::ci::appends_only(dir, &spec.base, &spec.sha, f));
    let adds_secret = files
        .iter()
        .filter(|f| {
            rules
                .secret_lists
                .iter()
                .any(|g| crate::approvals::matches(g, f))
        })
        .any(|f| adds_secret(dir, &spec.base, f));
    let facts = crate::approvals::Facts {
        deploys_worker: crate::cobot::scope::workers(dir, Some(files))
            .map_or(true, |w| !w.is_empty()),
        appends_only,
        adds_secret,
    };
    crate::approvals::classify(rules, &spec.scope, files, facts)
}

/// The approval rules on the base branch; none when it has none, or they don't read.
fn base_rules(spec: &JobSpec, dir: &Path) -> Option<crate::approvals::Rules> {
    let text = crate::cobot::ci::base_file(dir, &spec.base, crate::approvals::RULES)?;
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
    let before = crate::cobot::ci::base_file(dir, base, path)
        .and_then(|t| names(&t))
        .unwrap_or_default();
    crate::approvals::lengthens(&before, &head)
}

/// The head's violations of the base's policies, judged by `sentinel`, `main`'s
/// `pigeon-sentinel`; a judge that can't answer is a violation, never a pass; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §2.
fn policy(
    spec: &JobSpec,
    dir: &Path,
    files: &[String],
    sentinel: &Path,
    log: &mut String,
) -> Result<Vec<String>, String> {
    let sum = std::env::var(crate::cobot::ship_wire::SENTINEL_SHA256_VAR).unwrap_or_default();
    let _ = writeln!(log, "sentinel: {} (sha256 {sum})", sentinel.display());
    match base_policies(spec, dir)? {
        Some(p) => judge_with(sentinel, spec, dir, files, &p),
        None => Ok(Vec::new()),
    }
}

/// The violations `binary`, a `pigeon-sentinel`, finds in the head at `dir` against `policies`,
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
    let pr = [
        ("PR_TITLE", spec.title.clone()),
        ("PR_BODY", spec.plan.clone()),
        ("PR_NUMBER", spec.number.to_string()),
        ("PR_FILES", files.join("\n")),
        ("PR_BRANCH", spec.branch.clone()),
        ("PR_SCOPE", spec.scope.join("\n")),
    ];
    crate::sentinel::check(binary, dir, policies, Some(&pr))
        .map(|r| r.violations)
        .map_err(|e| e.to_string())
}

/// The base's policy directory, as the base's manifest names it, written under the checkout's
/// `.git`; none when the base names none. A change is never judged by rules it edits; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §2.
fn base_policies(spec: &JobSpec, dir: &Path) -> Result<Option<PathBuf>, String> {
    let manifest = crate::instance::MANIFEST;
    let Some(text) = crate::cobot::ci::base_file(dir, &spec.base, manifest) else {
        return Ok(None);
    };
    let v: Value =
        serde_saphyr::from_str(&text).map_err(|e| format!("{}'s {manifest}: {e}", spec.base))?;
    let Some(rel) = v["paths"]["policies"].as_str() else {
        return Ok(None);
    };
    let out = dir.join(".git").join("pigeon-base-policies");
    let _ = std::fs::remove_dir_all(&out);
    crate::cobot::ci::base_tree(dir, &spec.base, rel, &out).map_err(|e| e.to_string())?;
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
        |r| (200, json!(r)),
    )
}
