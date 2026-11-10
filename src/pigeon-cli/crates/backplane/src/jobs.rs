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
use crate::control::{DEPLOY, JobSpec, MIRROR, MIRROR_TO, MIRROR_TOKEN, Outcome, Report, tail};
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

/// Runs `spec` in `dir`: checks out its head, then the bound job or its command.
#[must_use]
pub fn run(spec: &JobSpec, dir: &Path) -> Report {
    // A merge whose report was lost may already be on the base; finding it there is the merge.
    if spec.job == MERGE
        && let Ok(Some(commit)) =
            cobot::ci::merged_already(dir, &spec.remote, &spec.token, &spec.base, &spec.sha)
    {
        return Report {
            log: format!("{} is already on {} as {commit}\n", spec.sha, spec.base),
            merged: Some(commit),
            ..report(spec, Outcome::Passed, "")
        };
    }
    if spec.job == MIRROR {
        return mirror(spec, dir);
    }
    let _ = std::fs::remove_dir_all(dir);
    if let Err(e) = cobot::ci::checkout(
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
        return bound(spec, dir);
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
    if let Err(e) = cobot::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return report(spec, Outcome::Failed, &e.to_string());
    }
    let workspace = dir.join(cobot::scope::WORKSPACE);
    let (ok, log) = cobot::ci::exec(&workspace, &spec.argv, cobot::ci::TIMEOUT, &spec.token);
    report(
        spec,
        if ok { Outcome::Passed } else { Outcome::Failed },
        &log,
    )
}

/// The mirror job: `main` and the tags, fast-forward and add-only, to GitHub in a fresh bare
/// repository; see `.claude/decisions/2026-11-08-development-github-mirror-on-merge.md`.
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
    if let Err(e) = cobot::git::bare(&staged) {
        return failed(e.to_string());
    }
    match cobot::git::mirror_from(
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
        Err(e) => failed(e.to_string().replace(to_token.as_str(), "***")),
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
    if let Err(e) = cobot::ci::fetch_tags(dir, &spec.remote, &spec.token) {
        return failed(e.to_string(), "");
    }
    let files = match &spec.since {
        Some(since) => cobot::ci::changed_since(dir, since, &spec.sha).ok(),
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
    let sites = cobot::scope::deploys(dir, files).map_err(|e| e.to_string())?;
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
    let r =
        run_with(&instance, &request, |k| spec.env.get(k).cloned()).map_err(|e| e.to_string())?;
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
        let _ = writeln!(message, "Change-Approved-By: {by}");
    }
    let _ = writeln!(message, "Change-Head: {}", spec.sha);
    match cobot::ci::squash_onto(
        dir,
        &spec.remote,
        &spec.token,
        &spec.base,
        &spec.branch,
        &spec.sha,
        &message,
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
    match cobot::ci::restack(
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
                cobot::ci::Relation::Current => Relation::Current,
                cobot::ci::Relation::Disjoint => Relation::Disjoint,
                cobot::ci::Relation::Clean => Relation::Clean,
                cobot::ci::Relation::Conflicting => Relation::Conflicting,
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
fn bound(spec: &JobSpec, dir: &Path) -> Report {
    let files = match cobot::ci::changed(dir, &spec.base, &spec.sha) {
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
    let tiers = tiers(spec, dir, &files);
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
    let violations = match policy(spec, dir, &files) {
        Ok(v) => v,
        Err(e) => vec![e],
    };
    for v in &violations {
        let _ = writeln!(log, "violation: {v}");
    }
    let checks = if violations.is_empty() {
        match cobot::scope::jobs(dir, &files) {
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

/// The head's approval tiers, by the rules on the base branch; none when the base has none; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §2.
fn tiers(spec: &JobSpec, dir: &Path, files: &[String]) -> Option<crate::approvals::Tiers> {
    let text = cobot::ci::base_file(dir, &spec.base, crate::approvals::RULES)?;
    let rules: crate::approvals::Rules = serde_saphyr::from_str(&text).ok()?;
    let decisions = rules.decisions.clone().unwrap_or_default();
    let appends_only = !decisions.is_empty()
        && files
            .iter()
            .filter(|f| f.starts_with(&decisions))
            .all(|f| cobot::ci::appends_only(dir, &spec.base, &spec.sha, f));
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
        deploys_worker: cobot::scope::workers(dir, Some(files)).map_or(true, |w| !w.is_empty()),
        appends_only,
        adds_secret,
    };
    Some(crate::approvals::classify(
        &rules,
        &spec.scope,
        files,
        facts,
    ))
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
    let before = cobot::ci::base_file(dir, base, path)
        .and_then(|t| names(&t))
        .unwrap_or_default();
    crate::approvals::lengthens(&before, &head)
}

fn policy(spec: &JobSpec, dir: &Path, files: &[String]) -> Result<Vec<String>, String> {
    let instance = Instance::load(dir).map_err(|e| e.to_string())?;
    let mut args = Map::new();
    for (k, v) in [
        ("branch", spec.branch.clone()),
        ("number", spec.number.to_string()),
        ("title", spec.title.clone()),
        ("body", spec.plan.clone()),
        ("scope", spec.scope.join("\n")),
        ("files", files.join("\n")),
    ] {
        args.insert(k.into(), Value::String(v));
    }
    let request = Request {
        operation: "policy.check".into(),
        args,
    };
    run_with(&instance, &request, |_| None)
        .map(|r| r.violations)
        .map_err(|e| e.to_string())
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
