//! A turn held by no merge is handed back; see the 2026-11-11 amendment to
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md`.

use backplane::changes::{Change, Effect, Event, GATE, JobStatus, MERGE, RESTACK, State};
use backplane::control;
use backplane::merges::{MergeQueue, Queued, SETTLE, holds_turn};
use backplane::runs::MAX_ATTEMPTS;
use serde_json::{Value, json};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";

fn apply(c: &mut Change, e: Event) -> Vec<Effect> {
    c.apply(e).expect("applies")
}

fn job(name: &str, ok: Option<bool>) -> Event {
    let kind = ok.map_or("started", |_| "finished");
    ev(&json!({ "type": kind, "job": name, "sha": A, "ok": ok }))
}

fn ev(v: &Value) -> Event {
    serde_json::from_value(v.clone()).expect("an event")
}

/// Change 175 on head `A`, approved, `fmt` passed and `test` left to its merge gate.
fn approved_175() -> Change {
    let mut c = Change::propose(175, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
    let check = |n: &str| json!({ "name": n, "argv": ["cargo", n] });
    let t = |tier: &str| json!({ "tier": tier, "rule": "routine" });
    let tiers = json!({ "authorize": t("confirm"), "merge": t("code"), "close": t("confirm") });
    for e in [
        json!({ "type": "approve_plan", "by": "owner", "at": 1 }),
        json!({ "type": "pushed", "sha": A }),
        json!({ "type": "bounded", "sha": A, "violations": [], "checks": [check("fmt")],
                "full": [check("test")], "tiers": tiers }),
        json!({ "type": "started", "job": "fmt", "sha": A }),
        json!({ "type": "finished", "job": "fmt", "sha": A, "ok": true }),
        json!({ "type": "approve", "by": "owner", "sha": A, "at": 5 }),
    ] {
        apply(&mut c, ev(&e));
    }
    assert_eq!(c.state, State::Approved);
    c
}

/// Its merge gate passes and its merge job is queued.
fn gated(c: &mut Change) {
    let gate = format!("{GATE}test");
    let merge = Event::Merge { by: "owner".into() };
    assert_eq!(apply(c, merge), [Effect::Start(gate.clone())]);
    apply(c, job(&gate, None));
    assert_eq!(
        apply(c, job(&gate, Some(true))),
        [Effect::Start(MERGE.into())]
    );
}

#[test]
fn a_merge_lost_on_every_start_holds_no_turn_and_merges_again_only_through_the_queue() {
    let (mut c, mut q) = (approved_175(), MergeQueue::default());
    assert_eq!(q.enqueue(175, "owner", 100), Ok(Queued::Start));
    gated(&mut c);
    // The git front door answers 502: each start is recorded Started, then Lost.
    for n in 1..=MAX_ATTEMPTS {
        apply(&mut c, job(MERGE, None));
        apply(
            &mut c,
            ev(&json!({ "type": "lost", "job": MERGE, "sha": A })),
        );
        assert_eq!(holds_turn(&c), n < MAX_ATTEMPTS, "attempt {n}");
    }
    assert_eq!(c.state, State::Approved, "its approval stands");
    let merge = c.jobs.iter().find(|j| j.name == MERGE).expect("merge job");
    assert_eq!(merge.status, JobStatus::Failed);
    let why = control::release_reason(&c);
    assert!(why.contains("lost 3 times"), "{why}");
    let rerun = c
        .apply(Event::Rerun { job: MERGE.into() })
        .expect_err("refused");
    assert!(rerun.0.contains("change_merge 175"), "{}", rerun.0);

    // The queue still holds 175, and asks about its holder only once the turn settles.
    assert!(
        q.enqueue(175, "owner", 101).is_err(),
        "already being merged"
    );
    assert_eq!(q.holder(100 + SETTLE - 1), None);
    assert_eq!(q.holder(100 + SETTLE), Some(175));
    assert_eq!(q.done(175, 100 + SETTLE), None, "handed back");

    // change_merge on 175 then starts its merge job at once: its gate already passed.
    assert_eq!(q.enqueue(175, "owner", 100 + SETTLE), Ok(Queued::Start));
    let merge = Event::Merge { by: "owner".into() };
    assert_eq!(apply(&mut c, merge), [Effect::Start(MERGE.into())]);
    assert!(holds_turn(&c));
}

#[test]
fn a_merge_on_its_way_keeps_its_turn() {
    assert!(!holds_turn(&approved_175()), "approved, nothing asked");
    let merge = || Event::Merge { by: "owner".into() };
    let behind = || Event::Behind { main: MAIN.into() };

    let mut gate = approved_175();
    apply(&mut gate, merge());
    assert!(holds_turn(&gate), "a gate check queued, its merge parked");

    let mut restack = approved_175();
    assert_eq!(
        apply(&mut restack, behind()),
        [Effect::Start(RESTACK.into())]
    );
    assert!(holds_turn(&restack), "a restack queued");
    apply(&mut restack, merge());
    assert!(restack.merge_after.is_some(), "a merge parked");

    let mut merging = approved_175();
    gated(&mut merging);
    apply(&mut merging, job(MERGE, None));
    assert!(holds_turn(&merging), "a merge job running");
}
