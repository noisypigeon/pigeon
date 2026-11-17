//! `step` as tables of state, event, next state and effects, with no storage.

use super::*;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const M: &str = "dddddddddddddddddddddddddddddddddddddddd";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

fn run(c: Change, events: Vec<Event>) -> Change {
    events
        .into_iter()
        .fold(c, |c, e| step(&c, e).expect("steps").0)
}

fn proposed() -> Change {
    Change::propose(1, "x", "feat: x", "p", vec!["**".into()], "agent")
}

fn planned() -> Change {
    run(
        proposed(),
        vec![Event::ApprovePlan {
            by: "owner".into(),
            at: 1,
        }],
    )
}

fn pushed() -> Change {
    run(planned(), vec![Event::Pushed { sha: A.into() }])
}

fn bounded(full: Vec<Check>) -> Event {
    Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full,
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    }
}

fn checked() -> Change {
    run(pushed(), vec![bounded(Vec::new())])
}

fn approved() -> Change {
    run(
        checked(),
        vec![Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 2,
        }],
    )
}

fn merge() -> Event {
    Event::Merge { by: "owner".into() }
}

fn current() -> Event {
    Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Current,
            to: None,
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    }
}

/// `c` behind `main` with a merge waiting on it, its restack job running.
fn restacking(mut c: Change) -> Change {
    c.merge_after = Some("owner".into());
    run(
        c,
        vec![
            Event::Behind { main: M.into() },
            Event::Started {
                job: RESTACK.into(),
                sha: A.into(),
            },
        ],
    )
}

fn conflicting(mut c: Change) -> Change {
    c.conflict = Some(Conflicted {
        onto: M.into(),
        paths: vec!["a.rs".into()],
        proposal: None,
        reconcile: None,
    });
    c
}

fn status(c: &Change, name: &str) -> Option<JobStatus> {
    c.jobs.iter().find(|j| j.name == name).map(|j| j.status)
}

/// A case: its name, the change, the event, and the next state and effects, or none for a
/// refusal.
type Row = (&'static str, Change, Event, Option<(State, Vec<Effect>)>);

#[allow(clippy::too_many_lines)]
fn table() -> Vec<Row> {
    let start = |job: &str| Effect::Start(job.into());
    vec![
        (
            "a push before the plan's approval",
            proposed(),
            Event::Pushed { sha: A.into() },
            None,
        ),
        (
            "a plan approved by its proposer",
            proposed(),
            Event::ApprovePlan {
                by: "agent".into(),
                at: 1,
            },
            None,
        ),
        (
            "a push",
            planned(),
            Event::Pushed { sha: A.into() },
            Some((State::Pushed, vec![start(BOUND)])),
        ),
        (
            "a push that is no SHA",
            planned(),
            Event::Pushed { sha: "main".into() },
            None,
        ),
        (
            "a refused head",
            pushed(),
            Event::Bounded {
                sha: A.into(),
                violations: vec!["no".into()],
                checks: Vec::new(),
                full: Vec::new(),
                tiers: None,
                gate: None,
                patch: None,
                own: None,
            },
            Some((State::Refused, Vec::new())),
        ),
        (
            "a bounded head with a check",
            pushed(),
            Event::Bounded {
                sha: A.into(),
                violations: Vec::new(),
                checks: vec![Check {
                    name: "fmt".into(),
                    argv: Vec::new(),
                }],
                full: Vec::new(),
                tiers: None,
                gate: None,
                patch: None,
                own: None,
            },
            Some((State::Bounded, vec![start("fmt")])),
        ),
        (
            "a head approved by its proposer",
            checked(),
            Event::Approve {
                by: "agent".into(),
                sha: A.into(),
                at: 2,
            },
            None,
        ),
        (
            "an approval",
            checked(),
            Event::Approve {
                by: "owner".into(),
                sha: A.into(),
                at: 2,
            },
            Some((State::Approved, Vec::new())),
        ),
        (
            "a merge",
            approved(),
            merge(),
            Some((State::Approved, vec![start(MERGE)])),
        ),
        (
            "a close",
            approved(),
            Event::Close,
            Some((State::Closed, Vec::new())),
        ),
        (
            "a push after the close",
            run(approved(), vec![Event::Close]),
            Event::Pushed { sha: A.into() },
            None,
        ),
    ]
}

#[test]
fn each_event_steps_a_change_to_its_next_state_with_its_effects() {
    for (name, before, event, want) in table() {
        let got = step(&before, event);
        match want {
            Some((state, effects)) => {
                let (next, got) = got.unwrap_or_else(|r| panic!("{name}: refused: {r}"));
                assert_eq!((next.state, got), (state, effects), "{name}");
            }
            None => assert!(got.is_err(), "{name}: steps"),
        }
    }
}

#[test]
fn a_held_head_that_is_no_sha_is_dropped_and_the_restack_report_kept() {
    let mut c = run(
        restacking(approved()),
        vec![Event::Pushed { sha: "main".into() }],
    );
    c.merge_after = None;
    assert_eq!(c.held.as_deref(), Some("main"));
    let (c, effects) = step(&c, current()).expect("the report applies");
    assert_eq!(effects, Vec::new());
    assert_eq!(status(&c, RESTACK), Some(JobStatus::Passed));
    assert_eq!(
        (c.held, c.head.as_deref(), c.state, c.behind),
        (None, Some(A), State::Approved, None)
    );
}

#[test]
fn a_parked_merge_that_cannot_start_after_a_restack_is_dropped_and_the_report_kept() {
    let c = run(restacking(approved()), vec![merge()]);
    assert_eq!(c.merge_after.as_deref(), Some("owner"), "parked");
    let (c, effects) = step(&conflicting(c), current()).expect("the report applies");
    assert_eq!(effects, Vec::new(), "no merge starts");
    assert_eq!(c.merge_after, None);
    assert_eq!(status(&c, RESTACK), Some(JobStatus::Passed));
}

#[test]
fn a_parked_merge_that_cannot_start_after_its_gate_is_dropped_and_the_gate_kept() {
    let fmt = Check {
        name: "fmt".into(),
        argv: Vec::new(),
    };
    let gated = run(pushed(), vec![bounded(vec![fmt])]);
    let gated = run(
        gated,
        vec![
            Event::Approve {
                by: "owner".into(),
                sha: A.into(),
                at: 2,
            },
            merge(),
            Event::Started {
                job: "gate: fmt".into(),
                sha: A.into(),
            },
        ],
    );
    assert_eq!(
        gated.merge_after.as_deref(),
        Some("owner"),
        "waits for its gate"
    );
    let passed = Event::Finished {
        job: "gate: fmt".into(),
        sha: A.into(),
        ok: true,
    };
    let (c, effects) = step(&conflicting(gated), passed).expect("the gate settles");
    assert_eq!(effects, Vec::new(), "no merge starts");
    assert_eq!(c.merge_after, None);
    assert_eq!(status(&c, "gate: fmt"), Some(JobStatus::Passed));
    assert_eq!(status(&c, MERGE), None);
}

#[test]
fn a_refusal_leaves_the_change_as_it_was() {
    let mut before = pushed();
    before.jobs.clear();
    let tiers_before = before.tiers.clone();
    let event = Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: vec![Check {
            name: "fmt".into(),
            argv: Vec::new(),
        }],
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    };
    assert!(step(&before, event).is_err(), "no bound job");
    assert_eq!((before.full.len(), before.tiers), (0, tiers_before));
}

fn full_set() -> Vec<Check> {
    let check = |name: &str, argv: &[&str]| Check {
        name: name.into(),
        argv: argv.iter().map(|a| (*a).to_string()).collect(),
    };
    vec![
        check("clippy", &["cargo", "clippy", "--workspace"]),
        check("test", &["cargo", "test", "--workspace", "--locked"]),
    ]
}

/// An approved head whose bound job reported `gate`, with its merge asked for.
fn gated(gate: Option<crate::deploy::gate::Gate>) -> (Change, Vec<Effect>) {
    let bounded = Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: full_set(),
        tiers: None,
        gate,
        patch: None,
        own: None,
    };
    let c = run(
        pushed(),
        vec![
            bounded,
            Event::Approve {
                by: "owner".into(),
                sha: A.into(),
                at: 2,
            },
        ],
    );
    step(&c, merge()).expect("merges")
}

#[test]
fn a_scoped_gate_leaves_out_the_whole_test_suite_and_a_full_one_keeps_it() {
    use crate::deploy::gate::Gate;
    let start = |names: &[&str]| -> Vec<Effect> {
        names.iter().map(|n| Effect::Start((*n).into())).collect()
    };
    let (c, effects) = gated(Some(Gate::Scoped));
    assert_eq!(effects, start(&["gate: clippy"]), "scoped");
    assert_eq!(c.gate_why(), None);
    let (c, effects) = gated(Some(Gate::Full("3 packages".into())));
    assert_eq!(effects, start(&["gate: clippy", "gate: test"]), "full");
    assert_eq!(c.gate_why().as_deref(), Some("3 packages"));
    let (_, effects) = gated(None);
    assert_eq!(
        effects,
        start(&["gate: clippy", "gate: test"]),
        "a head no gate was reported for gates in full"
    );
}

#[test]
fn a_clean_restack_makes_the_gate_full_and_a_session_push_clears_it() {
    use crate::deploy::gate::Gate;
    let mut c = approved();
    c.gate = Some(Gate::Scoped);
    let c = restacking(c);
    let clean = Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let (c, _) = step(&c, clean).expect("the restack applies");
    assert_eq!(c.head.as_deref(), Some(B));
    assert!(
        c.gate_why()
            .is_some_and(|w| w.contains("main changed its files"))
    );
    let rebound = Event::Bounded {
        sha: B.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: Vec::new(),
        tiers: None,
        gate: Some(Gate::Scoped),
        patch: None,
        own: None,
    };
    let (c, _) = step(&c, rebound).expect("bounded");
    assert!(
        c.gate_why().is_some(),
        "a scoped report doesn't undo a clean restack"
    );
    let (c, _) = step(&c, Event::Pushed { sha: C.into() }).expect("a session push");
    assert_eq!(c.gate_full, None);
}

#[test]
fn a_full_run_asked_for_at_merge_holds_whatever_the_gate() {
    use crate::deploy::gate::Gate;
    let mut c = run(
        pushed(),
        vec![Event::Bounded {
            sha: A.into(),
            violations: Vec::new(),
            checks: Vec::new(),
            full: full_set(),
            tiers: None,
            gate: Some(Gate::Scoped),
            patch: None,
            own: None,
        }],
    );
    c = run(
        c,
        vec![Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 2,
        }],
    );
    let (c, effects) = step(&c, Event::MergeFull { by: "owner".into() }).expect("merges");
    assert_eq!(
        effects,
        vec![
            Effect::Start("gate: clippy".into()),
            Effect::Start("gate: test".into())
        ]
    );
    assert_eq!(
        c.gate_why().as_deref(),
        Some("owner asked for the full run")
    );
}

fn behind_main() -> Event {
    Event::Behind { main: M.into() }
}

fn restacks(effects: &[Effect]) -> bool {
    effects.contains(&Effect::Start(RESTACK.into()))
}

#[test]
fn a_head_checked_while_behind_waits_for_its_merge_turn_then_restacks() {
    let c = run(pushed(), vec![behind_main()]);
    assert_eq!(status(&c, RESTACK), None, "not while its checks run");
    let (c, effects) = step(&c, bounded(Vec::new())).expect("bounded");
    assert_eq!(c.state, State::Checked);
    assert!(!restacks(&effects), "no merge waits: {effects:?}");
    let approve = Event::Approve {
        by: "owner".into(),
        sha: A.into(),
        at: 1,
    };
    let (c, effects) = step(&c, approve).expect("approved");
    assert!(!restacks(&effects), "approving alone restacks nothing");
    let (c, effects) = step(&c, merge()).expect("its turn");
    assert!(restacks(&effects), "{effects:?}");
    assert_eq!(
        c.merge_after.as_deref(),
        Some("owner"),
        "the merge waits, parked"
    );
    assert_eq!(
        status(&c, MERGE),
        None,
        "no gate on a head that would be refused"
    );
}

#[test]
fn a_merge_behind_main_marks_others_behind_without_restacking_them() {
    let (c, effects) = step(&approved(), behind_main()).expect("behind");
    assert_eq!(c.behind.as_deref(), Some(M));
    assert_eq!(effects, Vec::new());
    assert_eq!(status(&c, RESTACK), None);
}

#[test]
fn a_head_checked_on_main_does_not_restack() {
    let (c, effects) = step(&pushed(), bounded(Vec::new())).expect("bounded");
    assert_eq!(c.state, State::Checked);
    assert!(!restacks(&effects), "{effects:?}");
}

#[test]
fn a_merge_refused_while_behind_restacks_carrying_the_approval() {
    use crate::approval::tiers::{Tier, Tiered, Tiers};
    let tiered = |tier| Tiered {
        tier,
        rule: "r".into(),
    };
    let mut c = approved();
    c.tiers = Some(Tiers {
        authorize: tiered(Tier::Confirm),
        merge: tiered(Tier::Code),
        close: tiered(Tier::Confirm),
    });
    let (c, effects) = step(&c, merge()).expect("merges");
    assert_eq!(effects, vec![Effect::Start(MERGE.into())]);
    let c = run(
        c,
        vec![
            Event::Started {
                job: MERGE.into(),
                sha: A.into(),
            },
            behind_main(),
        ],
    );
    assert_eq!(status(&c, RESTACK), None, "not while its merge runs");
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: "main has moved past its base".into(),
    };
    let (c, effects) = step(&c, refused).expect("the refusal applies");
    assert_eq!(status(&c, MERGE), Some(JobStatus::Failed));
    assert!(restacks(&effects), "{effects:?}");
    let carried = c
        .carried
        .as_ref()
        .map(|k| (k.by.as_str(), k.from.as_str(), k.merge));
    assert_eq!(carried, Some(("owner", A, Tier::Code)));
}

#[test]
fn a_merge_refused_on_main_does_not_restack() {
    let (c, _) = step(&approved(), merge()).expect("merges");
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: "no".into(),
    };
    let (c, effects) = step(&c, refused).expect("the refusal applies");
    assert_eq!((c.state, effects), (State::Approved, Vec::new()));
}

#[test]
fn a_restack_that_finds_the_patch_on_main_records_it_and_releases_the_merge() {
    let mut c = restacking(approved());
    c.merge_after = Some("owner".into());
    let landed = Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: None,
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: true,
        },
    };
    let (c, effects) = step(&c, landed).expect("the report applies");
    assert_eq!(effects, Vec::new());
    assert_eq!(c.landed.as_deref(), Some(A), "the head is recorded");
    assert_eq!(
        (c.behind.as_deref(), c.merge_after.as_deref()),
        (None, None)
    );
    assert_eq!(c.conflict, None, "not a conflict: nothing to reconcile");
    assert!(
        c.violations[0].contains("already on main"),
        "{:?}",
        c.violations
    );
    assert_eq!(c.state, State::Approved, "it waits for its close");
}

/// `c` with its restack job lost on every attempt.
fn restack_lost(mut c: Change) -> Change {
    for _ in 0..10 {
        if !matches!(
            status(&c, RESTACK),
            Some(JobStatus::Queued | JobStatus::Running)
        ) {
            break;
        }
        let lost = Event::Lost {
            job: RESTACK.into(),
            sha: A.into(),
        };
        c = step(&c, lost).expect("lost applies").0;
        if status(&c, RESTACK) == Some(JobStatus::Queued) {
            let started = Event::Started {
                job: RESTACK.into(),
                sha: A.into(),
            };
            c = step(&c, started).expect("started").0;
        }
    }
    c
}

#[test]
fn a_restack_lost_on_its_last_attempt_applies_its_held_push_and_says_why() {
    let c = restacking(approved());
    let (c, _) = step(&c, Event::Pushed { sha: C.into() }).expect("held");
    assert_eq!(c.held.as_deref(), Some(C));
    let c = restack_lost(c);
    assert_eq!(
        c.head.as_deref(),
        Some(C),
        "the held head applied, with its own jobs"
    );
    assert_eq!(status(&c, RESTACK), None);
    assert_eq!(c.held, None);
}

#[test]
fn a_restack_lost_on_its_last_attempt_gives_back_a_parked_merge() {
    let mut c = restacking(approved());
    c.merge_after = Some("owner".into());
    let c = restack_lost(c);
    assert_eq!(c.merge_after, None, "its turn goes back");
    assert!(
        c.violations
            .iter()
            .any(|v| v.contains("restack job's Container was lost")),
        "{:?}",
        c.violations
    );
    assert_eq!(c.head.as_deref(), Some(A), "nothing else moved");
}

#[test]
fn an_approval_carries_across_a_clean_restack_of_the_same_patch_only() {
    let clean = |same_patch: bool| Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let mut c = approved();
    c.tiers = serde_json::from_value(serde_json::json!({
        "authorize": { "rule": "routine", "tier": "confirm" },
        "merge": { "rule": "deploys", "tier": "code" },
        "close": { "rule": "close", "tier": "confirm" },
    }))
    .ok();
    assert!(c.tiers.is_some());
    let base = restacking(c);
    let (same, _) = step(&base, clean(true)).expect("restacked");
    assert_eq!(same.head.as_deref(), Some(B));
    assert!(
        same.carried.is_some(),
        "the same patch carries its approval"
    );
    let (changed, _) = step(&base, clean(false)).expect("restacked");
    assert_eq!(changed.carried, None, "a changed patch asks again");
}

#[test]
fn a_yielded_change_waits_for_its_change_and_is_freed_when_that_ends() {
    let c = approved()
        .yield_to(80, "s-a", "it lands the same file", 5)
        .expect("yields");
    let y = c.yielded.clone().expect("parked");
    assert_eq!((y.to, y.by.as_str(), y.at), (80, "s-a", 5));
    let refused = step(&c, merge()).expect_err("waits");
    assert!(refused.0.contains("yielded to !80"), "{}", refused.0);
    let c = c.unyield();
    assert_eq!(c.yielded, None);
    assert!(step(&c, merge()).is_ok(), "merges once freed");
    let own = approved().number;
    assert!(
        approved().yield_to(own, "s-a", "", 1).is_err(),
        "not to itself"
    );
    let mut closed = approved();
    closed.state = State::Closed;
    assert!(
        closed.yield_to(80, "s-a", "", 1).is_err(),
        "nothing to yield"
    );
}

#[test]
fn a_change_built_on_another_counts_from_its_branch_until_it_ends_then_stands_on_main() {
    let mut c = approved();
    c.after = Some(After {
        number: 80,
        branch: "change/80-x".into(),
        head: None,
    });
    assert_eq!(c.built_on(), None, "only while it waits for it");
    let c = c.yield_to(80, "s-a", "built on !80", 1).expect("yields");
    assert_eq!(c.built_on(), Some("change/80-x"));
    let other = c.clone().unyield().after_ended(81, Some(M.into()));
    assert_eq!(
        other.after.and_then(|a| a.head),
        None,
        "another change ended"
    );
    let c = c.unyield().after_ended(80, Some(M.into()));
    assert_eq!(c.after.as_ref().and_then(|a| a.head.as_deref()), Some(M));
    assert_eq!(c.built_on(), None);
    let restacked = Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let (c, _) = step(&restacking(c), restacked).expect("restacked");
    assert_eq!(c.after, None, "on main now");
    let old: Change =
        serde_json::from_value(serde_json::to_value(approved()).expect("json")).expect("reads");
    assert_eq!(old.after, None, "the version before reads as none");
}

#[test]
fn a_handed_change_is_the_taking_sessions() {
    let mut c = approved();
    c.session = Some("s-a".into());
    let c = c.hand_to("s-b");
    assert_eq!(c.session.as_deref(), Some("s-b"));
    assert_eq!(c.state, State::Approved, "nothing else moves");
}

#[test]
fn of_two_changes_forecast_to_conflict_the_one_less_far_along_yields() {
    use super::forecast::yields;
    assert!(
        yields((90, "bounded"), (80, "approved")),
        "theirs is further along"
    );
    assert!(!yields((90, "approved"), (80, "bounded")), "mine is");
    assert!(
        yields((90, "checked"), (80, "checked")),
        "as far along: the newer yields"
    );
    assert!(!yields((80, "checked"), (90, "checked")));
    assert!(
        yields((90, "pushed"), (80, "bounded")),
        "pushed and bounded rank alike"
    );
}

fn tiered(rule: &str) -> crate::approval::tiers::Tiers {
    let t = |r: &str| crate::approval::tiers::Tiered {
        tier: crate::approval::tiers::Tier::Code,
        rule: r.into(),
    };
    crate::approval::tiers::Tiers {
        authorize: t("routine"),
        merge: t(rule),
        close: t("close"),
    }
}

fn bounded_with(tiers: crate::approval::tiers::Tiers) -> Event {
    Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: Vec::new(),
        tiers: Some(tiers),
        gate: None,
        patch: None,
        own: None,
    }
}

#[test]
fn a_briefs_change_on_a_guarded_path_is_refused_at_bound() {
    let mut briefed = pushed();
    briefed.brief = Some("b4".into());
    let refused = run(
        briefed.clone(),
        vec![bounded_with(tiered("guarded: src/x/approve.rs"))],
    );
    assert_eq!(refused.state, State::Refused);
    assert!(
        refused.violations[0].contains("guarded: src/x/approve.rs"),
        "{:?}",
        refused.violations
    );
    let fine = run(briefed, vec![bounded_with(tiered("deploys"))]);
    assert_eq!(
        fine.state,
        State::Checked,
        "a Worker input is an executor's to change"
    );
    let planner = run(
        pushed(),
        vec![bounded_with(tiered("guarded: src/x/approve.rs"))],
    );
    assert_eq!(
        planner.state,
        State::Checked,
        "a change with no brief is the planner's"
    );
}

fn briefed_checked() -> Change {
    let mut c = checked();
    c.brief = Some("b4".into());
    c
}

fn approve_by(by: &str) -> Event {
    Event::Approve {
        by: by.into(),
        sha: A.into(),
        at: 3,
    }
}

#[test]
fn a_briefs_head_waits_for_its_planner_before_any_automatic_approval() {
    let c = briefed_checked();
    for auto in ["window:6", "mandate:2", "policy:ADR amendment"] {
        let refused = step(&c, approve_by(auto)).expect_err("waits for review");
        assert!(refused.0.contains("brief_review"), "{auto}: {}", refused.0);
    }
    let (owner, _) = step(&c, approve_by("owner")).expect("the owner may approve");
    assert_eq!(owner.state, State::Approved);
    assert!(
        step(
            &c,
            Event::Accept {
                by: "p".into(),
                sha: B.into()
            }
        )
        .is_err(),
        "not the head"
    );
    assert!(
        step(
            &checked(),
            Event::Accept {
                by: "p".into(),
                sha: A.into()
            }
        )
        .is_err(),
        "no brief"
    );
    let (accepted, _) = step(
        &c,
        Event::Accept {
            by: "p".into(),
            sha: A.into(),
        },
    )
    .expect("accepts");
    assert_eq!(accepted.accepted.as_deref(), Some(A));
    let (approved, _) = step(&accepted, approve_by("window:6")).expect("then a window approves");
    assert_eq!(approved.state, State::Approved);
}

#[test]
fn a_briefs_checked_head_is_in_review_until_accepted() {
    use super::stage::{Context, Needs, of};
    let c = briefed_checked();
    let s = of(&c, &Context::default());
    assert_eq!((s.stage.as_str(), s.needs), ("in-review", Needs::Agent));
    assert!(s.next.contains("b4"), "{}", s.next);
    let (accepted, _) = step(
        &c,
        Event::Accept {
            by: "p".into(),
            sha: A.into(),
        },
    )
    .expect("accepts");
    assert_eq!(of(&accepted, &Context::default()).stage, "checked");
}

#[test]
fn an_acceptance_follows_its_head_across_a_same_patch_restack_only() {
    let clean = |same_patch: bool| Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let mut c = approved();
    c.brief = Some("b4".into());
    c.accepted = Some(A.into());
    let base = restacking(c);
    let (same, _) = step(&base, clean(true)).expect("restacked");
    assert_eq!(
        same.accepted.as_deref(),
        Some(B),
        "the same patch keeps its acceptance"
    );
    assert!(!same.awaits_review());
    let (changed, _) = step(&base, clean(false)).expect("restacked");
    assert!(changed.awaits_review(), "a changed patch is reviewed again");
}

fn escalate(question: &str) -> Event {
    Event::Escalate {
        by: "https://claude.ai/code/executor".into(),
        question: question.into(),
        at: 9,
    }
}

#[test]
fn an_executor_escalates_only_a_briefs_change_and_its_planner_reopens_it() {
    use super::stage::{Context, of};
    assert!(step(&checked(), escalate("why?")).is_err(), "no brief");
    let c = briefed_checked();
    assert!(step(&c, escalate("  ")).is_err(), "says what it asks");
    let (up, _) = step(&c, escalate("the brief names no error type")).expect("escalates");
    assert_eq!(of(&up, &Context::default()).stage, "escalated");
    let mut accepted = up.clone();
    accepted.accepted = Some(A.into());
    let refused = step(&accepted, approve_by("window:6")).expect_err("no Auto while escalated");
    assert!(refused.0.contains("brief_review"), "{}", refused.0);
    let (open, _) = step(
        &up,
        Event::Reopen {
            by: "planner".into(),
        },
    )
    .expect("reopens");
    assert_eq!(open.escalated, None);
    assert_eq!(of(&open, &Context::default()).stage, "in-review");
}

#[test]
fn a_dropped_merge_is_kept_when_a_window_approves_the_restacked_head_again() {
    let changed = Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let checked_b = |c: Change| {
        run(
            c,
            vec![Event::Bounded {
                sha: B.into(),
                violations: Vec::new(),
                checks: Vec::new(),
                full: Vec::new(),
                tiers: None,
                gate: None,
                patch: None,
                own: None,
            }],
        )
    };
    let dropped = checked_b(
        step(&restacking(approved()), changed.clone())
            .expect("restacked")
            .0,
    );
    assert_eq!(dropped.state, State::Checked);
    assert_eq!(
        (dropped.approval.is_none(), dropped.merge_after.is_none()),
        (true, true)
    );
    let again = Event::Approve {
        by: "window:6".into(),
        sha: B.into(),
        at: 9,
    };
    let (approved_b, _) = step(&dropped, again.clone()).expect("the window approves again");
    let (merging, effects) = step(&approved_b, merge()).expect("the merge goes on");
    assert_eq!(merging.state, State::Approved);
    assert!(
        effects.contains(&Effect::Start(MERGE.into())),
        "{effects:?}"
    );
    let mut briefed = approved();
    briefed.brief = Some("b4".into());
    let held = checked_b(step(&restacking(briefed), changed).expect("restacked").0);
    assert!(
        step(&held, again).is_err(),
        "a brief's head still waits for its planner"
    );
}

/// `name`'s job on `c`.
fn job_of<'a>(c: &'a Change, name: &str) -> &'a Job {
    c.jobs.iter().find(|j| j.name == name).expect("the job")
}

/// `c` behind `main` with a merge waiting on it, its restack job queued.
fn restack_queued(mut c: Change) -> Change {
    c.merge_after = Some("owner".into());
    run(c, vec![Event::Behind { main: M.into() }])
}

fn start_failed(job: &str) -> Event {
    Event::StartFailed {
        job: job.into(),
        sha: A.into(),
        why: "Artifacts: minting a write token: 503".into(),
    }
}

fn started(job: &str) -> Event {
    Event::Started {
        job: job.into(),
        sha: A.into(),
    }
}

/// The waits of each `StartAfter` in `effects`, for `job`.
fn waits(effects: &[Effect], job: &str) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::StartAfter { job: j, secs } if j == job => Some(*secs),
            _ => None,
        })
        .collect()
}

#[test]
fn a_failed_start_keeps_the_job_queued_and_starts_it_again_after_a_backoff() {
    let mut c = restack_queued(approved());
    assert_eq!(status(&c, RESTACK), Some(JobStatus::Queued));
    let mut all = Vec::new();
    for _ in 0..7 {
        let (next, effects) = step(&c, start_failed(RESTACK)).expect("applies");
        all.extend(waits(&effects, RESTACK));
        c = next;
    }
    assert_eq!(all, vec![15, 30, 60, 120, 240, 240, 240]);
    let j = job_of(&c, RESTACK);
    assert_eq!(j.status, JobStatus::Queued, "it keeps the merge's turn");
    assert_eq!(j.attempts, 0, "no attempt is spent");
    assert_eq!(j.start_failures, 7);
    assert!(j.last_error.as_deref().is_some_and(|e| e.contains("503")));
    assert!(crate::change::queue::holds_turn(&c));
    let (c, _) = step(&c, started(RESTACK)).expect("starts");
    assert_eq!(job_of(&c, RESTACK).start_failures, 0);
    assert_eq!(job_of(&c, RESTACK).attempts, 1);
}

#[test]
fn eight_failed_starts_end_a_restack_as_failed_naming_the_error() {
    let mut c = restack_queued(approved());
    for _ in 0..8 {
        c = step(&c, start_failed(RESTACK)).expect("applies").0;
    }
    assert_eq!(c.merge_after, None, "its turn goes back");
    assert!(
        c.violations
            .iter()
            .any(|v| v.contains("failed to start 8 times") && v.contains("503")),
        "{:?}",
        c.violations
    );
}

#[test]
fn a_failed_start_of_a_running_job_or_on_another_head_changes_nothing() {
    let c = restack_queued(approved());
    let other = Event::StartFailed {
        job: RESTACK.into(),
        sha: B.into(),
        why: "x".into(),
    };
    assert_eq!(step(&c, other).expect("applies").0, c);
    let running = restacking(approved());
    assert_eq!(
        step(&running, start_failed(RESTACK)).expect("applies").0,
        running
    );
}

#[test]
fn a_lost_job_starts_again_after_30_then_60_seconds() {
    let mut c = restacking(approved());
    let mut all = Vec::new();
    for n in 0..2 {
        let lost = Event::Lost {
            job: RESTACK.into(),
            sha: A.into(),
        };
        let (next, effects) = step(&c, lost).expect("lost");
        all.extend(waits(&effects, RESTACK));
        c = next;
        assert!(job_of(&c, RESTACK).last_error.is_some());
        if n == 0 {
            c = step(&c, started(RESTACK)).expect("starts").0;
        }
    }
    assert_eq!(all, vec![30, 60]);
}

#[test]
fn a_job_queued_that_never_started_is_dispatched_again_and_a_started_one_is_not() {
    let c = restack_queued(approved());
    let (same, effects) = step(
        &c,
        Event::Rerun {
            job: RESTACK.into(),
        },
    )
    .expect("re-dispatched");
    assert_eq!(effects, vec![Effect::Start(RESTACK.into())]);
    assert_eq!(same, c, "nothing else changes");
    let running = restacking(approved());
    assert!(
        step(
            &running,
            Event::Rerun {
                job: RESTACK.into()
            }
        )
        .is_err()
    );
}

fn bounded_at(sha: &str, merge: crate::approval::tiers::Tier, patch: Option<&str>) -> Event {
    let mut tiers = tiered("routine");
    tiers.merge.tier = merge;
    Event::Bounded {
        sha: sha.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: Vec::new(),
        tiers: Some(tiers),
        gate: None,
        patch: patch.map(str::to_string),
        own: None,
    }
}

/// !464: approved on `5f1bccf`, then the session rebased and pushed `efd44b8`, the same patch.
fn approved_then_pushed(merge: crate::approval::tiers::Tier) -> Change {
    run(
        planned(),
        vec![
            Event::Pushed { sha: A.into() },
            bounded_at(A, merge, Some("p464")),
            Event::Approve {
                by: "owner".into(),
                sha: A.into(),
                at: 2,
            },
            Event::Pushed { sha: B.into() },
        ],
    )
}

#[test]
fn case_464_a_session_push_of_the_same_patch_carries_the_owners_approval() {
    use crate::approval::tiers::Tier;
    let c = approved_then_pushed(Tier::Code);
    assert_eq!(c.state, State::Pushed);
    assert_eq!(
        c.approval, None,
        "nothing is approved before the new head is checked"
    );
    let c = run(c, vec![bounded_at(B, Tier::Code, Some("p464"))]);
    assert_eq!(c.state, State::Approved);
    let a = c.approval.expect("carried");
    assert_eq!((a.by.as_str(), a.sha.as_deref()), ("owner", Some(B)));
    assert_eq!(c.carried, None);
}

#[test]
fn a_session_push_of_another_patch_asks_the_owner_again() {
    use crate::approval::tiers::Tier;
    let c = run(
        approved_then_pushed(Tier::Code),
        vec![bounded_at(B, Tier::Code, Some("other"))],
    );
    assert_eq!(
        (c.state, c.approval, c.carried),
        (State::Checked, None, None)
    );
}

#[test]
fn a_session_push_with_no_patch_reported_asks_the_owner_again() {
    use crate::approval::tiers::Tier;
    let c = run(
        approved_then_pushed(Tier::Code),
        vec![bounded_at(B, Tier::Code, None)],
    );
    assert_eq!(
        (c.state, c.approval, c.carried),
        (State::Checked, None, None)
    );
}

#[test]
fn a_session_push_whose_merge_tier_rose_asks_the_owner_again() {
    use crate::approval::tiers::Tier;
    let c = run(
        approved_then_pushed(Tier::Confirm),
        vec![bounded_at(B, Tier::Code, Some("p464"))],
    );
    assert_eq!(
        (c.state, c.approval, c.carried),
        (State::Checked, None, None)
    );
}

#[test]
fn a_push_before_any_approval_carries_nothing() {
    use crate::approval::tiers::Tier;
    let c = run(
        planned(),
        vec![
            Event::Pushed { sha: A.into() },
            bounded_at(A, Tier::Code, Some("p464")),
            Event::Pushed { sha: B.into() },
            bounded_at(B, Tier::Code, Some("p464")),
        ],
    );
    assert_eq!(
        (c.state, c.approval, c.carried),
        (State::Checked, None, None)
    );
}

/// No jobs.
const NONE: [String; 0] = [];

/// !506 yielded to !500 by forecast; !500 merges, the yield ends, and the merge queue asks again.
fn yield_ended_restack() -> (Change, Vec<Effect>) {
    let parked = approved()
        .yield_to(500, "forecast", "forecast: they conflict in a.rs", 3)
        .expect("yields");
    let behind = run(parked, vec![Event::Behind { main: M.into() }]);
    assert!(
        behind.job(RESTACK).is_none(),
        "no merge asked, so no restack yet"
    );
    step(&behind.unyield().after_ended(500, Some(B.into())), merge()).expect("merges")
}

#[test]
fn a_restack_queued_when_a_yield_ends_is_dispatched() {
    let (c, effects) = yield_ended_restack();
    assert_eq!(job_of(&c, RESTACK).status, JobStatus::Queued);
    assert_eq!(effects, vec![Effect::Start(RESTACK.into())]);
    assert_eq!(undispatched(&c, &[RESTACK], |_| false), NONE);
}

#[test]
fn a_queued_job_whose_start_was_lost_is_dispatched_again_and_one_run_elsewhere_is_not() {
    // An alarm saving its own pending over the merge event's start, as on !506.
    let (c, _) = yield_ended_restack();
    assert_eq!(undispatched(&c, &[], |_| false), vec![RESTACK.to_string()]);
    assert_eq!(undispatched(&c, &[], |j| j == RESTACK), NONE);
    assert_eq!(undispatched(&restacking(approved()), &[], |_| false), NONE);
}

#[test]
fn a_job_queued_with_no_attempt_for_five_minutes_restarts_and_its_time_with_it() {
    let (c, _) = yield_ended_restack();
    let (since, out) = stalled(&c, &std::collections::BTreeMap::new(), 100);
    assert_eq!(out, NONE);
    let (since, out) = stalled(&c, &since, 100 + STALL_SECS - 1);
    assert_eq!(out, NONE);
    let (since, out) = stalled(&c, &since, 100 + STALL_SECS);
    assert_eq!(out, vec![RESTACK.to_string()]);
    assert!(
        stalled(&c, &since, 101 + STALL_SECS).1.is_empty(),
        "its time restarted"
    );
    let failing = step(&c, start_failed(RESTACK)).expect("fails").0;
    assert!(
        stalled(&failing, &since, 10_000).1.is_empty(),
        "backing off"
    );
    assert!(
        stalled(&restacking(approved()), &since, 10_000)
            .0
            .is_empty()
    );
}

/// `c`, approved, holding the merge turn as !839 does: behind `main`, its restack moved its
/// head to `B` cleanly with its own lines changed, and its merge parked.
fn restacked_holder() -> Change {
    let changed = Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(B.into()),
            same_patch: false,
            conflicts: Vec::new(),
            onto: M.into(),
            context: Vec::new(),
            messages: String::new(),
            landed: false,
        },
    };
    let mut c = run(restacking(approved()), vec![changed]);
    c.number = 839;
    assert_eq!((c.state, c.head.as_deref()), (State::Pushed, Some(B)));
    assert!(super::queue::holds_turn(&c));
    c
}

/// The merge queue with !839 holding the turn and !840 waiting behind it.
fn turn_held_by_839() -> super::queue::MergeQueue {
    use super::queue::{MergeQueue, Queued, enqueue};
    let (q, at) = enqueue(&MergeQueue::default(), 839, "owner", 0).expect("!839");
    assert_eq!(at, Queued::Start);
    let (q, at) = enqueue(&q, 840, "owner", 0).expect("!840");
    assert_eq!(at, Queued::Behind(1));
    q
}

/// `after`, saved over `before` as the Worker saves it, gives its turn back: the next merge.
fn released(q: &super::queue::MergeQueue, before: &Change, after: &Change) -> Option<u32> {
    assert!(!crate::control::keeps_turn(Some(before), after));
    assert!(crate::control::leaves_merge_queue(Some(before), after));
    super::queue::done(q, after.number, 200).1.map(|e| e.number)
}

#[test]
fn replay_839_a_holder_refused_at_its_restacked_heads_bound_gives_its_turn_back() {
    use super::stage::{Context, Needs, of};
    let why = "decisions: `when` 2026-11-16 after 2026-11-17 is not ascending";
    let q = turn_held_by_839();
    let before = restacked_holder();
    let refusal = Event::Bounded {
        sha: B.into(),
        violations: vec![why.into()],
        checks: Vec::new(),
        full: Vec::new(),
        tiers: None,
        gate: None,
        patch: None,
        own: None,
    };
    let c = run(before.clone(), vec![refusal]);
    assert_eq!(c.state, State::Refused);
    assert_eq!(c.merge_after, None, "its parked merge goes back");
    assert!(!super::queue::holds_turn(&c));
    assert_eq!(crate::control::release_reason(&c), why);
    assert_eq!(released(&q, &before, &c), Some(840), "!840 starts");
    let s = of(&c, &Context::default());
    assert_eq!((s.stage.as_str(), s.needs), ("refused", Needs::Agent));
}

#[test]
fn a_holder_whose_restacked_heads_bound_job_never_starts_gives_its_turn_back() {
    let q = turn_held_by_839();
    let before = restacked_holder();
    let failed = |_| Event::StartFailed {
        job: BOUND.into(),
        sha: B.into(),
        why: "Artifacts: minting a write token: 503".into(),
    };
    let c = run(
        before.clone(),
        (0..MAX_START_FAILURES).map(failed).collect(),
    );
    assert_eq!(c.state, State::Refused);
    assert_eq!(c.merge_after, None);
    assert_eq!(released(&q, &before, &c), Some(840), "!840 starts");
}

/// !464's sequence with its merge asked by `asked` after the approval, if anyone: the change
/// before the bound job reports `B`, the same patch, and after.
fn carried_after(asked: Option<&str>) -> (Change, Change) {
    use crate::approval::tiers::Tier;
    let mut events = vec![
        Event::Pushed { sha: A.into() },
        bounded_at(A, Tier::Code, Some("p464")),
        Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 2,
        },
    ];
    events.extend(asked.map(|by| Event::MergeAsked { by: by.into() }));
    events.push(Event::Pushed { sha: B.into() });
    let before = run(planned(), events);
    let after = run(
        before.clone(),
        vec![bounded_at(B, Tier::Code, Some("p464"))],
    );
    (before, after)
}

#[test]
fn a_carried_approval_asks_again_the_merge_that_was_asked() {
    let (before, after) = carried_after(Some("window:3"));
    assert_eq!(after.state, State::Approved);
    assert_eq!(after.merge_after, None, "nothing parked it");
    assert_eq!(
        crate::change::queue::reask(
            Some(&before),
            &after,
            &crate::change::queue::MergeQueue::default()
        )
        .as_deref(),
        Some("window:3")
    );
}

#[test]
fn a_carried_approval_with_no_merge_asked_asks_nothing() {
    let (before, after) = carried_after(None);
    assert_eq!(after.state, State::Approved);
    assert_eq!(
        crate::change::queue::reask(
            Some(&before),
            &after,
            &crate::change::queue::MergeQueue::default()
        ),
        None
    );
}

#[test]
fn a_carried_approval_already_in_the_queue_is_not_asked_again() {
    let (before, after) = carried_after(Some("window:3"));
    let empty = crate::change::queue::MergeQueue::default();
    let (merging, _) = crate::change::queue::enqueue(&empty, 1, "window:3", 3).expect("starts");
    assert_eq!(
        crate::change::queue::reask(Some(&before), &after, &merging),
        None
    );
    let (other, _) = crate::change::queue::enqueue(&empty, 9, "owner", 3).expect("9 starts");
    let (waiting, _) = crate::change::queue::enqueue(&other, 1, "window:3", 4).expect("1 waits");
    assert_eq!(
        crate::change::queue::reask(Some(&before), &after, &waiting),
        None
    );
}

#[test]
fn an_approval_by_hand_is_not_asked_again_and_clears_the_ask() {
    let asked = run(
        checked(),
        vec![Event::MergeAsked {
            by: "window:3".into(),
        }],
    );
    assert_eq!(asked.merge_asked.as_deref(), Some("window:3"));
    let approved = run(
        asked.clone(),
        vec![Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 2,
        }],
    );
    assert_eq!(approved.merge_asked, None);
    assert_eq!(
        crate::change::queue::reask(
            Some(&asked),
            &approved,
            &crate::change::queue::MergeQueue::default()
        ),
        None
    );
}

#[test]
fn a_merge_records_who_asked_for_it() {
    let (c, _) = step(&approved(), merge()).expect("merges");
    assert_eq!(c.merge_asked.as_deref(), Some("owner"));
}

#[test]
fn a_session_push_is_counted_once_per_new_head() {
    let pushes = vec![
        Event::Pushed { sha: A.into() },
        Event::Pushed { sha: B.into() },
        Event::Pushed { sha: B.into() },
    ];
    let c = run(planned(), pushes);
    assert_eq!((c.pushes, c.own.as_deref()), (2, None));
}

#[test]
fn a_kept_place_returns_to_the_first_push_after_a_failure_or_to_the_same_own_lines() {
    use queue::{Back, RETURN, Returning};
    let back = |own: &str, pushes| Back {
        own: Some(own.into()),
        pushes,
    };
    let failed = Returning {
        ahead: vec![],
        fronted: true,
        own: Some("o1".into()),
        failed: Some(1),
        at: 100,
    };
    let first = Some("it is the first push since its head failed");
    assert_eq!(failed.returns(&back("x", 2), 200), first);
    assert_eq!(
        failed.returns(&back("o1", 1), 200),
        None,
        "a rerun is no push"
    );
    assert_eq!(failed.returns(&back("x", 3), 200), None, "a second push");
    assert_eq!(failed.returns(&back("x", 2), 100 + RETURN), None, "lapsed");
    let left = Returning {
        failed: None,
        ..failed
    };
    let same = Some("its own lines are unchanged");
    assert_eq!(left.returns(&back("o1", 5), 200), same);
    assert_eq!(left.returns(&back("o2", 2), 200), None);
    assert_eq!(left.returns(&Back::default(), 200), None);
}

/// Asked by window:9 with own lines `own` at `pushes` pushes.
fn asked(own: &str, pushes: u32) -> queue::Back {
    queue::Back {
        own: Some(own.into()),
        pushes,
    }
}

/// 1000 merging, 1001 to 1000 + `ahead` waiting, then `n` asked with own lines `o1` at 1 push.
fn waiting_with(n: u32, ahead: u32) -> queue::MergeQueue {
    let mut q = queue::MergeQueue::default();
    for i in 0..=ahead {
        q = queue::enqueue(&q, 1000 + i, "window:9", 0)
            .expect("queued")
            .0;
    }
    queue::enqueue_back(&q, n, "window:9", &asked("o1", 1), 0)
        .expect("queued")
        .0
}

fn line(q: &queue::MergeQueue) -> Vec<u32> {
    q.waiting.iter().map(|e| e.number).collect()
}

#[test]
fn a_change_that_left_returns_behind_those_that_were_ahead_only_with_its_own_lines() {
    let q = waiting_with(9, 2);
    let (q, _) = queue::enqueue(&q, 2000, "window:9", 0).expect("queued");
    let (q, next) = queue::left(&q, 9, None, 100);
    assert_eq!(next, None, "1000 still merges");
    let (q, next) = queue::done(&q, 1000, 110);
    assert_eq!(next.map(|e| e.number), Some(1001));
    let (same, queued) =
        queue::enqueue_back(&q, 9, "window:9", &asked("o1", 2), 200).expect("asked");
    assert_eq!(queued, queue::Queued::Behind(2));
    assert_eq!(line(&same), [1002, 9, 2000]);
    let (_, queued) = queue::enqueue_back(&q, 9, "window:9", &asked("o2", 2), 200).expect("asked");
    assert_eq!(queued, queue::Queued::Behind(3), "its own lines changed");
}

#[test]
fn a_fronted_change_whose_head_failed_returns_to_the_front_once() {
    use queue::Queued::Behind;
    let ask = |q: &queue::MergeQueue, a: &queue::Back, at: u64| {
        queue::enqueue_back(q, 821, "window:9", a, at)
            .expect("asked")
            .1
    };
    let q = waiting_with(821, 2);
    let (q, _) = queue::front(&q, 821).expect("fronted");
    let (q, next) = queue::done(&q, 1000, 10);
    assert_eq!(next.map(|e| e.number), Some(821));
    let (q, next) = queue::left(&q, 821, Some(1), 100);
    assert_eq!(next.map(|e| e.number), Some(1001));
    let (q, _) = queue::enqueue(&q, 2000, "window:9", 100).expect("queued");
    assert_eq!(
        ask(&q, &asked("o1", 1), 200),
        Behind(3),
        "a rerun is no push"
    );
    assert_eq!(ask(&q, &asked("x", 3), 200), Behind(3), "a second push");
    assert_eq!(
        ask(&q, &asked("x", 2), 100 + queue::RETURN),
        Behind(3),
        "lapsed"
    );
    let (q, queued) = queue::enqueue_back(&q, 821, "window:9", &asked("x", 2), 200).expect("asked");
    assert_eq!(queued, Behind(1), "the first push after it failed");
    assert!(q.returning.is_empty(), "used once");
    // It fails again: its next fix queues at the back.
    let (q, next) = queue::done(&q, 1001, 300);
    assert_eq!(next.map(|e| e.number), Some(821));
    let (q, next) = queue::left(&q, 821, Some(2), 400);
    assert_eq!(next.map(|e| e.number), Some(1002));
    assert!(q.returning.is_empty(), "no second kept place");
    assert_eq!(ask(&q, &asked("y", 3), 500), Behind(2));
}

#[test]
fn a_busy_or_dropped_push_is_transient() {
    for e in [
        "remote: artifacts_git_receive_pack_busy\nerror: failed to push some refs",
        "error: RPC failed; HTTP 502 curl 22 The requested URL returned error: 502",
        "fatal: unable to access 'https://x/': The requested URL returned error: 503",
        "error: RPC failed; HTTP 429",
        "error: RPC failed; HTTP 504",
        "send-pack: unexpected disconnect while reading sideband packet",
        "fatal: unable to access 'https://x/': Recv failure: Connection reset by peer",
        "fatal: early EOF",
        " ! [remote rejected] main -> main (artifacts_git_receive_pack_busy)",
    ] {
        assert!(push::transient(e), "{e}");
    }
}

#[test]
fn a_push_refused_for_good_is_not_transient() {
    for e in [
        " ! [rejected]        main -> main (fetch first)",
        " ! [rejected]        main -> main (non-fast-forward)",
        "error: RPC failed; HTTP 502\n ! [rejected] main -> main (fetch first)",
        "main has moved past change/9-x's base; rebase onto main and push",
        "remote: Permission to x denied\nfatal: HTTP 403",
        "",
    ] {
        assert!(!push::transient(e), "{e}");
    }
}

#[test]
fn a_merge_push_refused_for_now_merges_again_without_a_restack() {
    let (c, _) = step(&approved(), merge()).expect("merges");
    let refused = Event::MergeFailed {
        sha: A.into(),
        reason: format!("git push: x:refs/heads/main {TRANSIENT_PUSH} 5 times: busy"),
    };
    let (c, effects) = step(&c, refused).expect("the refusal applies");
    assert_eq!(effects, vec![Effect::Start(MERGE.into())]);
    assert_eq!(status(&c, MERGE), Some(JobStatus::Queued));
    assert_eq!(status(&c, RESTACK), None);
    assert_eq!((c.state, c.carried.is_none()), (State::Approved, true));
    assert!(queue::holds_turn(&c));
}
