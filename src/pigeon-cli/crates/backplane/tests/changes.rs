//! A change's lifecycle; see
//! `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md` §1.

#[path = "common/step.rs"]
mod step;

use backplane::changes::{
    BOUND, Change, Check, Effect, Event, JobStatus, MERGE, MIRROR, State, branch, is_sha,
};
use backplane::control;
use backplane::runs::MAX_ATTEMPTS;
use step::Apply;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn check(name: &str) -> Check {
    Check {
        name: name.into(),
        argv: vec!["cargo".into(), name.into()],
    }
}

fn planned() -> Change {
    let mut c = Change::propose(
        7,
        "change-model",
        "feat(backplane): x",
        "the plan",
        vec!["src/**".into()],
        "agent",
    );
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("approves");
    c
}

/// A change pushed at `sha` and bounded, with checks `fmt` and `test` running.
fn checking(sha: &str) -> Change {
    let mut c = planned();
    c.apply(Event::Pushed { sha: sha.into() }).expect("pushed");
    c.apply(Event::Started {
        job: BOUND.into(),
        sha: sha.into(),
    })
    .expect("starts");
    let effects = c
        .apply(Event::Bounded {
            sha: sha.into(),
            violations: vec![],
            checks: vec![check("fmt"), check("test")],
            full: vec![],
            tiers: None,
            gate: None,
        })
        .expect("bounded");
    assert_eq!(
        effects,
        [Effect::Start("fmt".into()), Effect::Start("test".into())]
    );
    c
}

fn pass(c: &mut Change, job: &str, sha: &str) {
    c.apply(Event::Started {
        job: job.into(),
        sha: sha.into(),
    })
    .expect("starts");
    c.apply(Event::Finished {
        job: job.into(),
        sha: sha.into(),
        ok: true,
    })
    .expect("finishes");
}

#[test]
fn a_change_goes_from_proposal_to_merge() {
    let mut c = checking(A);
    assert_eq!(c.state, State::Bounded);
    pass(&mut c, "fmt", A);
    assert_eq!(c.state, State::Bounded);
    pass(&mut c, "test", A);
    assert_eq!(c.state, State::Checked);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 9,
    })
    .expect("approves");
    assert_eq!(c.state, State::Approved);
    c.apply(Event::Merged {
        sha: A.into(),
        commit: None,
    })
    .expect("merges");
    assert_eq!(c.state, State::Merged);
    assert!(
        c.apply(Event::Pushed { sha: B.into() }).is_err(),
        "a merged change is done"
    );
}

#[test]
fn a_plan_is_approved_before_a_push_and_never_by_its_proposer() {
    let mut c = Change::propose(1, "x", "t", "p", vec![], "agent");
    assert!(c.apply(Event::Pushed { sha: A.into() }).is_err());
    let err = c
        .apply(Event::ApprovePlan {
            by: "agent".into(),
            at: 1,
        })
        .unwrap_err();
    assert!(err.0.contains("other than its proposer"), "{err}");
    assert_eq!(c.state, State::Proposed);
}

#[test]
fn a_new_push_cancels_the_old_jobs_and_drops_the_approval() {
    let mut c = checking(A);
    pass(&mut c, "fmt", A);
    pass(&mut c, "test", A);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 9,
    })
    .expect("approves");
    let effects = c.apply(Event::Pushed { sha: B.into() }).expect("pushed");
    assert_eq!(effects, [Effect::Start(BOUND.into())]);
    assert_eq!(c.state, State::Pushed);
    assert_eq!(c.approval, None);
    assert_eq!(c.jobs.len(), 1);
    let mut d = checking(A);
    d.apply(Event::Started {
        job: "fmt".into(),
        sha: A.into(),
    })
    .expect("starts");
    let effects = d.apply(Event::Pushed { sha: B.into() }).expect("pushed");
    assert_eq!(
        effects[0],
        Effect::Cancel(vec!["fmt".into(), "test".into()])
    );
    assert_eq!(
        d.apply(Event::Pushed { sha: B.into() }).expect("again"),
        [],
        "the same head twice is one push"
    );
}

#[test]
fn results_for_an_old_head_are_ignored() {
    let mut c = checking(A);
    c.apply(Event::Pushed { sha: B.into() }).expect("pushed");
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: A.into(),
        ok: false,
    })
    .expect("ignored");
    c.apply(Event::Bounded {
        sha: A.into(),
        violations: vec!["x".into()],
        checks: vec![],
        full: vec![],
        tiers: None,
        gate: None,
    })
    .expect("ignored");
    assert_eq!(c.state, State::Pushed);
}

#[test]
fn an_approval_covers_only_the_checked_head() {
    let mut c = checking(A);
    assert!(
        c.apply(Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 9
        })
        .is_err(),
        "not checked yet"
    );
    pass(&mut c, "fmt", A);
    pass(&mut c, "test", A);
    let stale = c
        .apply(Event::Approve {
            by: "owner".into(),
            sha: B.into(),
            at: 9,
        })
        .unwrap_err();
    assert!(
        stale.0.contains("is not change/7-change-model's head"),
        "{stale}"
    );
    let own = c
        .apply(Event::Approve {
            by: "agent".into(),
            sha: A.into(),
            at: 9,
        })
        .unwrap_err();
    assert!(own.0.contains("other than its proposer"), "{own}");
    assert!(
        c.apply(Event::Merged {
            sha: A.into(),
            commit: None
        })
        .is_err(),
        "unapproved"
    );
}

#[test]
fn a_refused_head_runs_no_checks_and_a_failure_stops_the_rest() {
    let mut c = planned();
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    let effects = c
        .apply(Event::Bounded {
            sha: A.into(),
            violations: vec!["outside `scope`: x".into()],
            checks: vec![check("fmt")],
            full: vec![],
            tiers: None,
            gate: None,
        })
        .expect("bounded");
    assert_eq!(effects, []);
    assert_eq!(c.state, State::Refused);
    assert_eq!(c.violations, ["outside `scope`: x"]);
    let mut d = checking(A);
    d.apply(Event::Started {
        job: "fmt".into(),
        sha: A.into(),
    })
    .expect("starts");
    let effects = d
        .apply(Event::Finished {
            job: "fmt".into(),
            sha: A.into(),
            ok: false,
        })
        .expect("fails");
    assert_eq!(effects, [Effect::Cancel(vec!["test".into()])]);
    assert_eq!(d.state, State::Failed);
}

#[test]
fn a_lost_job_retries_until_its_attempts_are_used() {
    let mut c = checking(A);
    for n in 1..=MAX_ATTEMPTS {
        c.apply(Event::Started {
            job: "fmt".into(),
            sha: A.into(),
        })
        .expect("starts");
        let effects = c
            .apply(Event::Lost {
                job: "fmt".into(),
                sha: A.into(),
            })
            .expect("lost");
        if n < MAX_ATTEMPTS {
            assert_eq!(effects, [Effect::Start("fmt".into())]);
        }
    }
    assert_eq!(c.state, State::Failed);
    assert_eq!(
        c.jobs.iter().find(|j| j.name == "fmt").map(|j| j.status),
        Some(JobStatus::Failed)
    );
}

#[test]
fn a_rerun_restarts_a_failed_job_and_those_it_cancelled() {
    let mut c = checking(A);
    c.apply(Event::Started {
        job: "fmt".into(),
        sha: A.into(),
    })
    .expect("starts");
    c.apply(Event::Finished {
        job: "fmt".into(),
        sha: A.into(),
        ok: false,
    })
    .expect("fails");
    assert!(c.apply(Event::Rerun { job: "nope".into() }).is_err());
    let effects = c.apply(Event::Rerun { job: "fmt".into() }).expect("reruns");
    assert_eq!(
        effects,
        [Effect::Start("fmt".into()), Effect::Start("test".into())]
    );
    assert_eq!(c.state, State::Bounded);
    pass(&mut c, "fmt", A);
    pass(&mut c, "test", A);
    assert_eq!(c.state, State::Checked);
}

#[test]
fn a_change_with_no_checks_is_checked_once_bounded() {
    let mut c = planned();
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Bounded {
        sha: A.into(),
        violations: vec![],
        checks: vec![],
        full: vec![],
        tiers: None,
        gate: None,
    })
    .expect("bounded");
    assert_eq!(c.state, State::Checked);
}

#[test]
fn branches_and_shas_are_parsed_strictly() {
    assert_eq!(
        branch("refs/heads/change/7-change-model"),
        Some((7, "change-model".into()))
    );
    assert_eq!(branch("change/12-x"), Some((12, "x".into())));
    for bad in [
        "main",
        "change/07-x",
        "change/7-",
        "change/7-X",
        "change/x-y",
        "change/7-a--b",
    ] {
        assert_eq!(branch(bad), None, "{bad}");
    }
    assert!(is_sha(A));
    assert!(!is_sha("ABC"));
    let mut c = planned();
    assert!(c.apply(Event::Pushed { sha: "main".into() }).is_err());
    c.apply(Event::Close).expect("closes");
    assert_eq!(c.state, State::Closed);
}

#[test]
fn a_change_round_trips_its_storage_form() {
    let c = checking(A);
    let text = serde_json::to_string(&c).expect("serialize");
    let back: Change = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back, c);
}

#[test]
fn a_merge_is_asked_for_once_approved_and_a_failure_keeps_it_approved() {
    let mut c = checking(A);
    assert!(
        c.apply(Event::Merge { by: "owner".into() }).is_err(),
        "not approved yet"
    );
    pass(&mut c, "fmt", A);
    pass(&mut c, "test", A);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 9,
    })
    .expect("approves");
    assert_eq!(
        c.apply(Event::Merge { by: "owner".into() })
            .expect("merges"),
        [Effect::Start(MERGE.into())]
    );
    assert!(
        c.apply(Event::Merge { by: "owner".into() }).is_err(),
        "one merge at a time"
    );
    c.apply(Event::Started {
        job: MERGE.into(),
        sha: A.into(),
    })
    .expect("starts");
    c.apply(Event::MergeFailed {
        sha: A.into(),
        reason: "main has moved".into(),
    })
    .expect("fails");
    assert_eq!(c.state, State::Approved);
    assert_eq!(c.violations, ["main has moved"]);
    assert_eq!(
        c.apply(Event::Merge { by: "owner".into() }).expect("again"),
        [Effect::Start(MERGE.into())]
    );
    c.apply(Event::Started {
        job: MERGE.into(),
        sha: A.into(),
    })
    .expect("starts");
    c.apply(Event::Merged {
        sha: A.into(),
        commit: Some(B.into()),
    })
    .expect("merged");
    assert_eq!(c.state, State::Merged);
    assert_eq!(c.merged_as.as_deref(), Some(B));
    assert_eq!(
        c.jobs.iter().find(|j| j.name == MERGE).map(|j| j.status),
        Some(JobStatus::Passed)
    );
}

#[test]
fn a_new_push_clears_the_old_heads_tiers() {
    let mut c = Change::propose(10, "x", "feat: x", "plan", vec!["**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("approves");
    c.apply(Event::Pushed {
        sha: "a".repeat(40),
    })
    .expect("pushed");
    c.tiers = Some(backplane::approvals::Tiers {
        authorize: backplane::approvals::Tiered {
            tier: backplane::approvals::Tier::Code,
            rule: "r".into(),
        },
        merge: backplane::approvals::Tiered {
            tier: backplane::approvals::Tier::Confirm,
            rule: "r".into(),
        },
        close: backplane::approvals::Tiered {
            tier: backplane::approvals::Tier::Confirm,
            rule: "r".into(),
        },
    });
    c.apply(Event::Pushed {
        sha: "b".repeat(40),
    })
    .expect("pushed again");
    assert_eq!(c.tiers, None);
}

/// A change pushed at `sha` and bounded on a Worker that mirrors, with `checks`.
fn mirrored(sha: &str, checks: &[&str]) -> (Change, Vec<Effect>) {
    let mut c = planned();
    c.apply(Event::Pushed { sha: sha.into() }).expect("pushed");
    c.apply(Event::Started {
        job: BOUND.into(),
        sha: sha.into(),
    })
    .expect("starts");
    let effects = c
        .apply(control::with_mirror(Event::Bounded {
            sha: sha.into(),
            violations: vec![],
            checks: checks.iter().map(|n| check(n)).collect(),
            full: vec![],
            tiers: None,
            gate: None,
        }))
        .expect("bounded");
    (c, effects)
}

#[test]
fn the_mirror_job_never_gates_a_change() {
    let (mut c, effects) = mirrored(A, &["fmt"]);
    assert_eq!(
        effects,
        [Effect::Start("fmt".into()), Effect::Start(MIRROR.into())]
    );
    pass(&mut c, "fmt", A);
    assert_eq!(c.state, State::Checked, "an open mirror job holds nothing");
    c.apply(Event::Started {
        job: MIRROR.into(),
        sha: A.into(),
    })
    .expect("starts");
    for _ in 0..MAX_ATTEMPTS {
        c.apply(Event::Lost {
            job: MIRROR.into(),
            sha: A.into(),
        })
        .expect("lost");
        let _ = c.apply(Event::Started {
            job: MIRROR.into(),
            sha: A.into(),
        });
    }
    assert_eq!(c.state, State::Checked, "nor does a failed one");

    let (mut c, effects) = mirrored(B, &[]);
    assert_eq!(effects, [Effect::Start(MIRROR.into())]);
    assert_eq!(c.state, State::Checked);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: B.into(),
        at: 9,
    })
    .expect("approves");
    let before = c.clone();
    let effects = c
        .apply(Event::Merged {
            sha: B.into(),
            commit: None,
        })
        .expect("merges");
    assert_eq!(effects, [Effect::Cancel(vec![MIRROR.into()])]);
    assert!(
        !control::unmirror_due(Some(&before), &c),
        "a mirror job never started pushed nothing"
    );
}

#[test]
fn a_refused_head_never_queues_the_mirror_job() {
    let mut c = planned();
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    let effects = c
        .apply(control::with_mirror(Event::Bounded {
            sha: A.into(),
            violations: vec!["outside its scope".into()],
            checks: vec![],
            full: vec![],
            tiers: None,
            gate: None,
        }))
        .expect("refused");
    assert_eq!(effects, []);
    assert_eq!(c.state, State::Refused);
    assert!(c.jobs.iter().all(|j| j.name != MIRROR));
    assert!(c.apply(Event::Rerun { job: MIRROR.into() }).is_err());
}

#[test]
fn a_mirrored_change_is_unmirrored_once_merged_or_closed() {
    let (mut c, _) = mirrored(A, &[]);
    pass(&mut c, MIRROR, A);
    let open = c.clone();
    c.apply(Event::Close).expect("closes");
    assert!(control::unmirror_due(Some(&open), &c));
    let closed = c.clone();
    c.apply(Event::Close).expect("closes again");
    assert!(!control::unmirror_due(Some(&closed), &c), "only once");

    let (mut c, _) = mirrored(B, &[]);
    pass(&mut c, MIRROR, B);
    c.apply(Event::Approve {
        by: "owner".into(),
        sha: B.into(),
        at: 9,
    })
    .expect("approves");
    let approved = c.clone();
    c.apply(Event::Merged {
        sha: B.into(),
        commit: None,
    })
    .expect("merges");
    assert!(control::unmirror_due(Some(&approved), &c));
    assert!(!control::unmirror_due(Some(&approved), &approved));
}

#[test]
fn a_failed_mirror_job_runs_again_while_the_change_is_open() {
    let (mut c, _) = mirrored(A, &["fmt"]);
    c.apply(Event::Started {
        job: MIRROR.into(),
        sha: A.into(),
    })
    .expect("starts");
    assert_eq!(
        c.apply(Event::Rerun { job: MIRROR.into() }).expect("open"),
        [],
        "an open mirror job is left to run"
    );
    c.apply(Event::Finished {
        job: MIRROR.into(),
        sha: A.into(),
        ok: false,
    })
    .expect("fails");
    assert_eq!(c.state, State::Bounded, "fmt still runs");
    assert_eq!(
        c.apply(Event::Rerun { job: MIRROR.into() })
            .expect("reruns"),
        [Effect::Start(MIRROR.into())]
    );
    assert_eq!(c.state, State::Bounded, "nothing else moves");
    let fmt = c.jobs.iter().find(|j| j.name == "fmt").expect("fmt");
    assert_eq!(fmt.status, JobStatus::Queued);
}
