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

/// `c` behind `main`, its restack job running.
fn restacking(c: Change) -> Change {
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
    let c = run(
        restacking(approved()),
        vec![Event::Pushed { sha: "main".into() }],
    );
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
fn gated(gate: Option<crate::core::deploy::gate::Gate>) -> (Change, Vec<Effect>) {
    let bounded = Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: Vec::new(),
        full: full_set(),
        tiers: None,
        gate,
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
    use crate::core::deploy::gate::Gate;
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
    use crate::core::deploy::gate::Gate;
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
    use crate::core::deploy::gate::Gate;
    let mut c = run(
        pushed(),
        vec![Event::Bounded {
            sha: A.into(),
            violations: Vec::new(),
            checks: Vec::new(),
            full: full_set(),
            tiers: None,
            gate: Some(Gate::Scoped),
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
fn a_head_checked_while_behind_restacks_then() {
    let c = run(pushed(), vec![behind_main()]);
    assert_eq!(status(&c, RESTACK), None, "not while its checks run");
    let (c, effects) = step(&c, bounded(Vec::new())).expect("bounded");
    assert_eq!(c.state, State::Checked);
    assert!(restacks(&effects), "{effects:?}");
    assert_eq!(status(&c, RESTACK), Some(JobStatus::Queued));
    assert_eq!(c.carried, None, "nothing approved to carry");
}

#[test]
fn a_head_whose_last_check_passes_while_behind_restacks_then() {
    let fmt = Event::Bounded {
        sha: A.into(),
        violations: Vec::new(),
        checks: vec![Check {
            name: "fmt".into(),
            argv: Vec::new(),
        }],
        full: Vec::new(),
        tiers: None,
        gate: None,
    };
    let c = run(
        pushed(),
        vec![
            fmt,
            Event::Started {
                job: "fmt".into(),
                sha: A.into(),
            },
            behind_main(),
        ],
    );
    let passed = Event::Finished {
        job: "fmt".into(),
        sha: A.into(),
        ok: true,
    };
    let (c, effects) = step(&c, passed).expect("settles");
    assert_eq!(c.state, State::Checked);
    assert!(restacks(&effects), "{effects:?}");
}

#[test]
fn a_head_checked_on_main_does_not_restack() {
    let (c, effects) = step(&pushed(), bounded(Vec::new())).expect("bounded");
    assert_eq!(c.state, State::Checked);
    assert!(!restacks(&effects), "{effects:?}");
}

#[test]
fn a_merge_refused_while_behind_restacks_carrying_the_approval() {
    use crate::core::approval::tiers::{Tier, Tiered, Tiers};
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
