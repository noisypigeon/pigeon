//! A head restacked for its merge turn runs its full gate beside its checks; see the 2026-11-15
//! merge-turn amendment to `.claude/decisions/2026-11-15-development-code-checks-by-reach.md`.

#[path = "common/step.rs"]
mod step;

use backplane::approvals::{Tier, Tiered, Tiers};
use backplane::changes::{
    Change, Check, Effect, Event, GATE, MERGE, RESTACK, Relation, Restacked, State,
};
use step::Apply;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";
const WINDOW: &str = "window:9";

fn check(name: &str, argv: &[&str]) -> Check {
    Check {
        name: name.into(),
        argv: argv.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn tiers() -> Tiers {
    let t = |tier| Tiered {
        tier,
        rule: "routine".into(),
    };
    Tiers {
        authorize: t(Tier::Confirm),
        merge: t(Tier::Code),
        close: t(Tier::Confirm),
    }
}

fn bounded(sha: &str) -> Event {
    Event::Bounded {
        sha: sha.into(),
        violations: vec![],
        checks: vec![check("fmt", &["cargo", "fmt"])],
        full: vec![check("test", &["cargo", "test", "--workspace"])],
        tiers: Some(tiers()),
        gate: None,
        patch: None,
    }
}

fn ran(c: &mut Change, job: &str, sha: &str, ok: bool) -> Vec<Effect> {
    let started = Event::Started {
        job: job.into(),
        sha: sha.into(),
    };
    c.apply(started).expect("started");
    let finished = Event::Finished {
        job: job.into(),
        sha: sha.into(),
        ok,
    };
    c.apply(finished).expect("finished")
}

/// Approved on `A`, then `main` moves and its turn comes: a clean restack to `C`, bounded.
fn at_turn(same_patch: bool) -> Change {
    let mut c = Change::propose(7, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: WINDOW.into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(bounded(A)).expect("bounded");
    ran(&mut c, "fmt", A, true);
    c.apply(Event::Approve {
        by: WINDOW.into(),
        sha: A.into(),
        at: 5,
    })
    .expect("approved");
    c.apply(Event::Behind { main: MAIN.into() })
        .expect("behind");
    let restack = c.apply(Event::Merge { by: WINDOW.into() }).expect("turn");
    assert_eq!(restack, [Effect::Start(RESTACK.into())]);
    c.apply(Event::Restacked {
        sha: A.into(),
        restack: Restacked {
            relation: Relation::Clean,
            to: Some(C.into()),
            same_patch,
            conflicts: vec![],
            onto: MAIN.into(),
            context: vec![],
            messages: String::new(),
            landed: false,
        },
    })
    .expect("restacked");
    assert!(c.gate_why().is_some(), "a clean restack's gate is full");
    let gate = format!("{GATE}test");
    let effects = c.apply(bounded(C)).expect("bounded");
    for job in ["fmt", gate.as_str()] {
        assert!(effects.contains(&Effect::Start(job.into())), "{job} starts");
    }
    c
}

#[test]
fn replays_546_the_carried_approval_starts_the_merge_as_the_gate_passes() {
    let mut c = at_turn(true);
    assert_eq!(ran(&mut c, "fmt", C, true), []);
    let merge = ran(&mut c, &format!("{GATE}test"), C, true);
    assert_eq!(c.state, State::Approved);
    assert_eq!(merge, [Effect::Start(MERGE.into())]);
}

#[test]
fn replays_598_the_windows_approval_starts_the_merge_with_no_gate_after_it() {
    let mut c = at_turn(false);
    ran(&mut c, &format!("{GATE}test"), C, true);
    assert_eq!(ran(&mut c, "fmt", C, true), []);
    assert_eq!(c.state, State::Checked);
    let approved = c
        .apply(Event::Approve {
            by: WINDOW.into(),
            sha: C.into(),
            at: 9,
        })
        .expect("approved");
    assert_eq!(approved, []);
    let merge = c.apply(Event::Merge { by: WINDOW.into() }).expect("merge");
    assert_eq!(merge, [Effect::Start(MERGE.into())]);
}

#[test]
fn a_failing_early_gate_fails_the_head() {
    let mut c = at_turn(true);
    ran(&mut c, &format!("{GATE}test"), C, false);
    assert_eq!(c.state, State::Failed);
}

#[test]
fn a_session_push_with_no_merge_parked_queues_no_gate() {
    let mut c = Change::propose(8, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    c.apply(Event::ApprovePlan {
        by: WINDOW.into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    let effects = c.apply(bounded(A)).expect("bounded");
    assert_eq!(effects, [Effect::Start("fmt".into())]);
}
