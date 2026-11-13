//! Open changes as eviction candidates; see
//! `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4.

#[path = "common/step.rs"]
mod step;

use backplane::core::change::eviction::{Facts, Holds, Session, candidate, kind};

const NOW: u64 = 1_000_000;

fn facts(slug: &str, session: Option<Session>) -> Facts {
    Facts {
        number: 79,
        slug: slug.into(),
        state: "pushed".into(),
        head: Some("a".repeat(40)),
        updated: NOW - 9_000,
        session,
        holds: Holds::default(),
    }
}

fn gone(for_: u64) -> Session {
    Session {
        label: "s-a".into(),
        live: false,
        gone_for: Some(for_),
    }
}

#[test]
fn a_change_with_a_gone_session_carries_every_field() {
    let c = candidate(&facts("x", Some(gone(7_300))), NOW);
    let want = [
        ("number", "79"),
        ("kind", "change"),
        ("state", "pushed"),
        ("pushed", "true"),
        ("session_live", "false"),
        ("session_gone_for", "7300"),
        ("idle", "9000"),
        ("pinned", "false"),
        ("approval_open", "false"),
        ("queued", "false"),
    ];
    for (k, v) in want {
        assert_eq!(c.get(k).map(String::as_str), Some(v), "{k}");
    }
    assert_eq!(c.len(), want.len());
}

#[test]
fn generated_changes_are_named_by_their_slug() {
    assert_eq!(kind("reconcile-79"), "reconcile");
    assert_eq!(kind("review-61-59"), "review");
    assert_eq!(kind("reconciler"), "change");
    assert_eq!(kind("reconcile-"), "change");
    assert_eq!(kind("feat-x"), "change");
}

#[test]
fn a_live_session_is_gone_for_nothing_and_an_unknown_one_leaves_the_field_out() {
    let live = Some(Session {
        label: "s-a".into(),
        live: true,
        gone_for: None,
    });
    let c = candidate(&facts("x", live), NOW);
    assert_eq!(c["session_live"], "true");
    assert_eq!(c["session_gone_for"], "0");
    let unknown = Some(Session {
        label: "s-a".into(),
        live: false,
        gone_for: None,
    });
    let c = candidate(&facts("x", unknown), NOW);
    assert_eq!(c["session_live"], "false");
    assert!(
        !c.contains_key("session_gone_for"),
        "no policy needing it grants"
    );
}

#[test]
fn a_change_without_a_session_is_gone_as_long_as_it_is_idle() {
    let mut f = facts("x", None);
    f.head = None;
    f.state = "proposed".into();
    let c = candidate(&f, NOW);
    assert_eq!(c["session_live"], "false");
    assert_eq!(c["session_gone_for"], "9000");
    assert_eq!(c["pushed"], "false");
}

#[test]
fn what_holds_a_change_is_carried() {
    let mut f = facts("x", Some(gone(1)));
    f.holds = Holds {
        pinned: true,
        approval_open: true,
        queued: true,
    };
    let c = candidate(&f, NOW);
    assert_eq!(
        (
            c["pinned"].as_str(),
            c["approval_open"].as_str(),
            c["queued"].as_str()
        ),
        ("true", "true", "true")
    );
}

mod acting {
    use super::step::Apply;
    use backplane::changes::{Change, Event, State};
    use backplane::core::change::eviction::{PER_RUN, incomplete, pick, remains};
    use serde_json::json;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn proposed() -> Change {
        Change::propose(
            79,
            "x",
            "feat(x): y",
            "Do x.\nThen y.",
            vec!["a/**".into()],
            "agent",
        )
    }

    fn pushed() -> Change {
        let mut c = proposed();
        c.apply(Event::ApprovePlan {
            by: "owner".into(),
            at: 1,
        })
        .expect("authorized");
        c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
        c
    }

    #[test]
    fn what_remains_follows_where_the_change_stopped() {
        let r = remains(&proposed());
        assert_eq!(r.left, ["the whole plan: nothing was pushed"]);
        assert_eq!(r.done[0], "plan: Do x.");
        assert!(r.resume.contains("`resumes: 79`"), "{}", r.resume);
        assert!(!incomplete(&proposed()), "only a plan");
        let p = pushed();
        assert!(incomplete(&p));
        let r = remains(&p);
        assert!(r.done.iter().any(|d| d.contains(A)), "{:?}", r.done);
        let mut behind = p.clone();
        behind.behind = Some("main".into());
        assert_eq!(remains(&behind).left, ["rebase onto main and push"]);
    }

    #[test]
    fn an_eviction_is_recorded_then_closed_keeping_its_head() {
        let c = pushed().evict("policy:eviction-session-gone", 9);
        let e = c.evicted.clone().expect("recorded");
        assert_eq!(
            (e.policy.as_str(), e.at, e.head.as_deref()),
            ("policy:eviction-session-gone", 9, Some(A))
        );
        let mut c = c;
        c.apply(Event::Close).expect("closes");
        assert_eq!(c.state, State::Closed);
        assert!(c.evicted.is_some(), "the record survives the close");
    }

    #[test]
    fn at_most_six_granted_lowest_first_and_only_from_fresh_verdicts() {
        let verdicts: Vec<serde_json::Value> = (1..=9)
            .rev()
            .map(|n| json!({ "number": n, "granted": if n == 5 { serde_json::Value::Null } else { json!("eviction-x") } }))
            .collect();
        let picked = pick(&verdicts, 100, 200);
        assert_eq!(picked.len(), PER_RUN);
        assert_eq!(
            picked.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [1, 2, 3, 4, 6, 7],
            "lowest first, the refused one skipped"
        );
        assert_eq!(pick(&verdicts, 100, 100 + 2 * 3600 + 1), vec![], "stale");
    }
}
