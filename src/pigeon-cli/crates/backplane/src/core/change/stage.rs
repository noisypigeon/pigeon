//! A change's stage: where it is, who acts next and what that is, derived from its record, its
//! place in the merge queue, the deploy carrying its merge and whether a window or mandate covers
//! it; see
//! `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §1.

use serde::{Deserialize, Serialize};

use super::queue::MergeQueue;
use super::types::{BOUND, Change, Job, JobStatus, MERGE, MIRROR, RESTACK, State};

/// Who acts next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Needs {
    /// The change's session.
    Agent,
    /// The owner, or a window or mandate that covers the change.
    Owner,
    /// The control plane.
    Platform,
    /// A code check's provider: GitHub Actions, Buildkite or a Container queue.
    Provider,
    /// Another change, named by `Stage::waits_on`.
    Change,
    /// Nobody.
    None,
}

/// Where a change is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// Its name, such as `checking 2/5` or `queued 2/3`.
    pub stage: String,
    /// Who acts next.
    pub needs: Needs,
    /// For `Needs::Change`: the change it waits on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<u32>,
    /// What acting next is, in a line.
    pub next: String,
    /// When it entered the stage, in Unix seconds: its last event that isn't a job's; the
    /// Worker fills it from the event log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
}

/// Where the deploy carrying a merged change stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "deploy", content = "why", rename_all = "snake_case")]
pub enum Deploy {
    /// Unknown: the merge is older than the pushes the pipeline keeps.
    #[default]
    Unknown,
    /// No deploy carrying it has started.
    Pending,
    /// A deploy carrying it runs.
    Running,
    /// A deploy carrying it passed.
    Passed,
    /// The last deploy carrying it failed, with why, and none has passed since.
    Failed(String),
}

/// What a change's stage is derived from, besides its record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// Its place in the merge queue.
    pub place: Option<Place>,
    /// For a merged change: the deploy carrying it.
    pub deploy: Deploy,
    /// The window or mandate that would allow its next authorization or approval, as
    /// `window:<id>` or `mandate:<id>`.
    pub covered: Option<String>,
}

/// A change's place in the merge queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Its merge holds the turn.
    Merging,
    /// It waits: turn `turn` of `of`, behind `holder`.
    Waiting {
        /// Its turn, from 1.
        turn: usize,
        /// How many merges are asked for, the running one included.
        of: usize,
        /// The change holding the turn, if any.
        holder: Option<u32>,
    },
}

/// Change `number`'s place in `q`, if it is in it.
#[must_use]
pub fn place(q: &MergeQueue, number: u32) -> Option<Place> {
    if q.merging.as_ref().is_some_and(|e| e.number == number) {
        return Some(Place::Merging);
    }
    let running = usize::from(q.merging.is_some());
    q.waiting
        .iter()
        .position(|e| e.number == number)
        .map(|i| Place::Waiting {
            turn: running + i + 1,
            of: running + q.waiting.len(),
            holder: q.merging.as_ref().map(|e| e.number),
        })
}

fn at(stage: impl Into<String>, needs: Needs, next: impl Into<String>) -> Stage {
    Stage {
        stage: stage.into(),
        needs,
        waits_on: None,
        next: next.into(),
        since: None,
    }
}

fn open(c: &Change, name: &str) -> bool {
    c.jobs
        .iter()
        .any(|j| j.name == name && matches!(j.status, JobStatus::Queued | JobStatus::Running))
}

fn failed(c: &Change, name: &str) -> bool {
    c.jobs
        .iter()
        .any(|j| j.name == name && j.status == JobStatus::Failed)
}

/// The checks that decide the head: every job but the bound job, the mirror, a restack and the
/// merge.
fn checks(c: &Change) -> impl Iterator<Item = &Job> {
    c.jobs
        .iter()
        .filter(|j| ![BOUND, MIRROR, RESTACK, MERGE].contains(&j.name.as_str()))
}

fn names<'a>(jobs: impl Iterator<Item = &'a Job>) -> String {
    jobs.map(|j| format!("`{}`", j.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A covered change waits on its session: a window or mandate acts only when asked.
fn ask(stage: &str, covered: Option<&String>, tool: &str, owner: &str) -> Stage {
    match covered {
        Some(by) => at(
            stage,
            Needs::Agent,
            format!("ask (`{tool}`); {by} allows it"),
        ),
        None => at(stage, Needs::Owner, owner),
    }
}

fn merged(deploy: &Deploy) -> Stage {
    match deploy {
        Deploy::Unknown => at("merged", Needs::None, "nothing"),
        Deploy::Pending => at(
            "deploying",
            Needs::Platform,
            "main's next deploy carries it",
        ),
        Deploy::Running => at("deploying", Needs::Platform, "the deploy carrying it runs"),
        Deploy::Passed => at("done", Needs::None, "nothing"),
        Deploy::Failed(why) => at(
            "deploy-failed",
            Needs::Owner,
            format!("deploy again (`deploy_retry`) once fixed: {why}"),
        ),
    }
}

/// Change `c`'s stage, given `cx`.
#[must_use]
pub fn of(c: &Change, cx: &Context) -> Stage {
    let branch = c.branch();
    let place = cx.place;
    let covered = cx.covered.as_ref();
    match c.state {
        State::Merged => merged(&cx.deploy),
        State::Closed => at("closed", Needs::None, "nothing"),
        State::Proposed if !c.asks.is_empty() => at(
            "proposed",
            Needs::Owner,
            "answer its asks and authorize the plan",
        ),
        State::Proposed => ask(
            "proposed",
            covered,
            "change_authorize",
            "authorize the plan",
        ),
        State::Planned => at("authorized", Needs::Agent, format!("push `{branch}`")),
        _ if c.conflict.is_some() => {
            let next = match c.conflict.as_ref().and_then(|x| x.reconcile) {
                Some(r) => format!("carry out !{r}, which reconciles it"),
                None => "rebase onto main and push".into(),
            };
            at("conflicted", Needs::Agent, next)
        }
        _ if open(c, RESTACK) => at("restacking", Needs::Platform, "the restack reports"),
        _ if open(c, MERGE) || place == Some(Place::Merging) => at(
            "merging",
            Needs::Platform,
            "the merge job squashes it onto main",
        ),
        State::Pushed => at(
            "bounding",
            Needs::Platform,
            "the bound job checks its bounds",
        ),
        State::Refused => {
            let why = c.violations.first().map_or("a policy", String::as_str);
            at("refused", Needs::Agent, format!("fix and push: {why}"))
        }
        State::Failed => {
            let jobs = names(checks(c).filter(|j| j.status == JobStatus::Failed));
            at(
                "failed",
                Needs::Agent,
                format!("fix and push: {jobs} failed"),
            )
        }
        State::Bounded => {
            let all: Vec<&Job> = checks(c).collect();
            if all.is_empty() {
                return at("checking", Needs::Provider, "the checks start");
            }
            let done = all.iter().filter(|j| j.status == JobStatus::Passed).count();
            let waiting = names(all.iter().copied().filter(|j| j.open()));
            at(
                format!("checking {done}/{}", all.len()),
                Needs::Provider,
                format!("the checks finish: {waiting}"),
            )
        }
        State::Checked if c.awaits_review() => at(
            "in-review",
            Needs::Agent,
            format!(
                "its planner reviews it against {} (`brief_review`)",
                c.brief.as_deref().unwrap_or_default()
            ),
        ),
        State::Checked => ask("checked", covered, "change_approve", "approve the head"),
        State::Approved => approved(c, place),
    }
}

/// An approved change's stage, given its place in the merge queue.
fn approved(c: &Change, place: Option<Place>) -> Stage {
    match place {
        Some(Place::Waiting { turn, of, .. }) if let Some(y) = &c.yielded => Stage {
            stage: format!("queued {turn}/{of}"),
            needs: Needs::Change,
            waits_on: Some(y.to),
            next: format!("!{} merges or closes; it keeps its place", y.to),
            since: None,
        },
        Some(Place::Waiting { turn, of, holder }) => Stage {
            stage: format!("queued {turn}/{of}"),
            needs: if holder.is_some() {
                Needs::Change
            } else {
                Needs::Platform
            },
            waits_on: holder,
            next: match holder {
                Some(h) => format!("!{h} merges or gives its turn back"),
                None => "its merge starts".into(),
            },
            since: None,
        },
        _ if c.merge_after.is_some() => at(
            "approved",
            Needs::Platform,
            "its merge is parked until its approval carries and its gate passes",
        ),
        _ if failed(c, MERGE) => {
            let why = c
                .violations
                .first()
                .map_or("the merge job failed", String::as_str);
            at(
                "approved",
                Needs::Agent,
                format!("ask to merge again (`change_merge`): {why}"),
            )
        }
        _ => at("approved", Needs::Agent, "ask to merge (`change_merge`)"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::queue::Entry;
    use super::super::types::{Approval, Conflicted};
    use super::*;

    fn change(state: State) -> Change {
        let mut c = Change::propose(7, "x", "feat: x", "p", vec!["**".into()], "agent");
        c.state = state;
        c
    }

    fn job(name: &str, status: JobStatus) -> Job {
        let mut j = Job::queued(name, Vec::new());
        j.status = status;
        j
    }

    fn with(place: Option<Place>) -> Context {
        Context {
            place,
            ..Context::default()
        }
    }

    fn entry(number: u32) -> Entry {
        Entry {
            number,
            by: "owner".into(),
            at: 1,
        }
    }

    #[test]
    fn each_state_names_who_acts() {
        let cases = [
            (State::Proposed, "proposed", Needs::Owner),
            (State::Planned, "authorized", Needs::Agent),
            (State::Pushed, "bounding", Needs::Platform),
            (State::Checked, "checked", Needs::Owner),
            (State::Approved, "approved", Needs::Agent),
            (State::Merged, "merged", Needs::None),
            (State::Closed, "closed", Needs::None),
        ];
        for (state, stage, needs) in cases {
            let s = of(&change(state), &Context::default());
            assert_eq!((s.stage.as_str(), s.needs), (stage, needs), "{state:?}");
        }
    }

    #[test]
    fn checking_counts_the_checks_alone() {
        let mut c = change(State::Bounded);
        c.jobs = vec![
            job(BOUND, JobStatus::Passed),
            job(MIRROR, JobStatus::Running),
            job("clippy", JobStatus::Passed),
            job("test", JobStatus::Running),
            job("wasm", JobStatus::Queued),
        ];
        let s = of(&c, &Context::default());
        assert_eq!(s.stage, "checking 1/3");
        assert_eq!(s.needs, Needs::Provider);
        assert_eq!(s.next, "the checks finish: `test`, `wasm`");
    }

    #[test]
    fn a_failure_names_its_checks() {
        let mut c = change(State::Failed);
        c.jobs = vec![
            job(BOUND, JobStatus::Passed),
            job("clippy", JobStatus::Failed),
            job("test", JobStatus::Passed),
        ];
        let s = of(&c, &Context::default());
        assert_eq!((s.stage.as_str(), s.needs), ("failed", Needs::Agent));
        assert_eq!(s.next, "fix and push: `clippy` failed");
    }

    #[test]
    fn a_refusal_names_its_first_violation() {
        let mut c = change(State::Refused);
        c.violations = vec!["outside its scope: a.rs".into(), "b".into()];
        assert_eq!(
            of(&c, &Context::default()).next,
            "fix and push: outside its scope: a.rs"
        );
    }

    #[test]
    fn a_conflict_points_at_its_reconciliation() {
        let mut c = change(State::Checked);
        c.conflict = Some(Conflicted {
            onto: "m".into(),
            paths: vec!["a.rs".into()],
            proposal: None,
            reconcile: Some(9),
        });
        let s = of(&c, &Context::default());
        assert_eq!((s.stage.as_str(), s.needs), ("conflicted", Needs::Agent));
        assert_eq!(s.next, "carry out !9, which reconciles it");
    }

    #[test]
    fn a_restack_or_merge_in_flight_is_the_platforms() {
        let mut c = change(State::Approved);
        c.jobs = vec![job(RESTACK, JobStatus::Running)];
        assert_eq!(of(&c, &Context::default()).stage, "restacking");
        c.jobs = vec![job(MERGE, JobStatus::Queued)];
        assert_eq!(of(&c, &Context::default()).stage, "merging");
        c.jobs.clear();
        assert_eq!(of(&c, &with(Some(Place::Merging))).stage, "merging");
    }

    #[test]
    fn a_queued_change_waits_on_the_holder() {
        let q = MergeQueue {
            merging: Some(entry(3)),
            waiting: [entry(5), entry(7)].into(),
            ..MergeQueue::default()
        };
        let p = place(&q, 7);
        assert_eq!(
            p,
            Some(Place::Waiting {
                turn: 3,
                of: 3,
                holder: Some(3)
            })
        );
        let s = of(&change(State::Approved), &with(p));
        assert_eq!(s.stage, "queued 3/3");
        assert_eq!((s.needs, s.waits_on), (Needs::Change, Some(3)));
        assert_eq!(place(&q, 3), Some(Place::Merging));
        assert_eq!(place(&q, 8), None);
    }

    #[test]
    fn a_queued_yielded_change_waits_on_the_change_it_yields_to() {
        let mut c = change(State::Approved);
        c = c
            .yield_to(9, "forecast", "they conflict", 1)
            .expect("yields");
        let p = Some(Place::Waiting {
            turn: 2,
            of: 3,
            holder: Some(3),
        });
        let s = of(&c, &with(p));
        assert_eq!(s.stage, "queued 2/3");
        assert_eq!((s.needs, s.waits_on), (Needs::Change, Some(9)));
        assert!(s.next.contains("keeps its place"), "{}", s.next);
    }

    #[test]
    fn an_approved_change_whose_merge_failed_asks_again() {
        let mut c = change(State::Approved);
        c.approval = Some(Approval {
            by: "window:1".into(),
            sha: Some("a".into()),
            at: 1,
        });
        c.jobs = vec![job(MERGE, JobStatus::Failed)];
        c.violations = vec!["main has moved past its base".into()];
        let s = of(&c, &Context::default());
        assert_eq!((s.stage.as_str(), s.needs), ("approved", Needs::Agent));
        assert_eq!(
            s.next,
            "ask to merge again (`change_merge`): main has moved past its base"
        );
    }

    #[test]
    fn a_parked_merge_waits_on_the_platform() {
        let mut c = change(State::Approved);
        c.merge_after = Some("owner".into());
        assert_eq!(of(&c, &Context::default()).needs, Needs::Platform);
    }

    #[test]
    fn a_covered_change_asks_its_window() {
        let cx = Context {
            covered: Some("window:2".into()),
            ..Context::default()
        };
        let s = of(&change(State::Checked), &cx);
        assert_eq!((s.stage.as_str(), s.needs), ("checked", Needs::Agent));
        assert_eq!(s.next, "ask (`change_approve`); window:2 allows it");
        let s = of(&change(State::Proposed), &cx);
        assert_eq!(s.next, "ask (`change_authorize`); window:2 allows it");
        let s = of(&change(State::Checked), &Context::default());
        assert_eq!(
            (s.needs, s.next.as_str()),
            (Needs::Owner, "approve the head")
        );
    }

    #[test]
    fn a_merged_change_follows_its_deploy() {
        let cases = [
            (Deploy::Unknown, "merged", Needs::None),
            (Deploy::Pending, "deploying", Needs::Platform),
            (Deploy::Running, "deploying", Needs::Platform),
            (Deploy::Passed, "done", Needs::None),
            (
                Deploy::Failed("smoke".into()),
                "deploy-failed",
                Needs::Owner,
            ),
        ];
        for (deploy, stage, needs) in cases {
            let cx = Context {
                deploy: deploy.clone(),
                ..Context::default()
            };
            let s = of(&change(State::Merged), &cx);
            assert_eq!((s.stage.as_str(), s.needs), (stage, needs), "{deploy:?}");
        }
    }

    #[test]
    fn a_bounded_head_without_its_checks_yet_is_checking() {
        let s = of(&change(State::Bounded), &Context::default());
        assert_eq!((s.stage.as_str(), s.needs), ("checking", Needs::Provider));
    }
}
