//! Reconciliation planned on conflict and on review; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §4.

#[path = "common/step.rs"]
mod step;

use backplane::changes::{Change, ConflictContext, Event, Relation, Restacked, State};
use backplane::reconcile::{ADR, adrs, conflict, review};
use step::Apply;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";
const OLD: &str = ".claude/decisions/2026-11-05-runtime-control-plane-worker.md";
const NEW: &str = ".claude/decisions/2026-11-09-development-merge-rules.md";

fn change(plan: &str) -> Change {
    let mut c = Change::propose(60, "x", "feat(x): y", plan, vec!["src/**".into()], "agent");
    c.session = Some("s-a".into());
    c
}

fn conflicting(paths: &[&str], messages: &str) -> Restacked {
    Restacked {
        relation: Relation::Conflicting,
        to: None,
        same_patch: false,
        conflicts: paths.iter().map(|p| (*p).to_string()).collect(),
        onto: MAIN.into(),
        context: paths
            .iter()
            .map(|p| ConflictContext {
                path: (*p).to_string(),
                commits: vec!["feat(z): w (!59)".into()],
                base_diff: "-a\n+b".into(),
                change_diff: "-a\n+c".into(),
            })
            .collect(),
        messages: messages.into(),
    }
}

#[test]
fn adrs_are_found_in_prose_once_each_in_date_order() {
    let text = format!("Per {NEW}. See `{OLD}` and {NEW}, and .claude/decisions/README.md.");
    assert_eq!(adrs(&text), [OLD, NEW]);
    assert_eq!(adrs("no decisions here"), Vec::<String>::new());
}

#[test]
fn a_conflict_citing_adrs_plans_an_amendment_to_the_later_one() {
    let c = change(&format!("Per {OLD}."));
    let p = conflict(
        &c,
        &conflicting(&["src/a.rs"], &format!("feat(z): w (!59)\n\nPer {NEW}.")),
    );
    assert_eq!(p.slug, "reconcile-60");
    assert_eq!(p.title, "fix: reconcile !60 with main");
    assert_eq!(p.scope, ["src/a.rs", NEW]);
    assert!(p.plan.contains(ADR), "cites the reconciliation ADR");
    assert!(p.plan.contains(&format!("append an amendment to {NEW}")));
    assert!(p.plan.contains("session s-a"));
    assert!(
        p.plan
            .contains("src/a.rs: main changed it in feat(z): w (!59).")
    );
    assert!(p.plan.contains("+b") && p.plan.contains("+c"), "both hunks");
}

#[test]
fn a_conflict_citing_no_adr_plans_a_code_fix() {
    let p = conflict(&change("no ADR"), &conflicting(&["src/a.rs"], "fix: z"));
    assert_eq!(p.scope, ["src/a.rs"]);
    assert!(p.plan.contains("Rule: fix the paths"));
}

#[test]
fn a_restack_conflict_holds_the_change_until_its_next_push() {
    let mut c = change(&format!("Per {OLD}."));
    c.apply(Event::ApprovePlan {
        by: "owner".into(),
        at: 1,
    })
    .expect("plan");
    c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
    c.apply(Event::Restacked {
        sha: A.into(),
        restack: conflicting(&["src/a.rs"], ""),
    })
    .expect("restacked");
    let held = c.conflict.as_ref().expect("conflicted");
    assert_eq!(
        (held.onto.as_str(), held.paths.clone()),
        (MAIN, vec!["src/a.rs".to_string()])
    );
    assert!(held.proposal.is_some(), "a reconciliation to propose");
    assert!(
        c.apply(Event::Approve {
            by: "owner".into(),
            sha: A.into(),
            at: 2
        })
        .is_err(),
        "a conflicted head isn't approved"
    );
    c.apply(Event::Pushed { sha: C.into() }).expect("pushed");
    assert_eq!(c.conflict, None);
    assert_eq!(c.state, State::Pushed);
}

#[test]
fn a_review_is_planned_only_when_the_open_change_cites_another_adr() {
    let globs = vec!["src/**".to_string()];
    let p = review(
        61,
        &format!("Per {OLD}."),
        59,
        &format!("Per {NEW}."),
        &globs,
    )
    .expect("a review");
    assert_eq!(p.slug, "review-61-59");
    assert_eq!(p.scope, [NEW]);
    assert!(p.plan.contains(ADR));
    assert!(p.plan.contains(&format!("citing {OLD}")));
    assert_eq!(
        review(
            61,
            &format!("Per {NEW}."),
            59,
            &format!("Per {NEW}."),
            &globs
        ),
        None,
        "the same ADR"
    );
    assert_eq!(
        review(61, "no ADR", 59, &format!("Per {NEW}."), &globs),
        None
    );
    assert_eq!(
        review(61, &format!("Per {OLD}."), 59, "no ADR", &globs),
        None
    );
}
