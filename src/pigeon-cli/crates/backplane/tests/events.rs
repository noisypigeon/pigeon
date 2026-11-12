//! The event log; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §1.

#[path = "common/step.rs"]
mod step;

use backplane::events::{Filter, KEEP, KEEP_FOR, Log};

#[test]
fn a_reader_gets_what_came_after_its_last_event_for_its_session_and_changes() {
    let mut log = Log::default();
    log.append(
        "change.checked",
        Some(60),
        Some("s-a".into()),
        "!60 checked",
        false,
        1,
    );
    log.append(
        "change.merged",
        Some(61),
        Some("s-b".into()),
        "!61 merged",
        false,
        2,
    );
    log.append("deploy.passed", None, None, "deploy passed", false, 3);
    log.append(
        "approval.pending",
        Some(60),
        Some("s-a".into()),
        "link",
        true,
        4,
    );
    let ids = |f: &Filter| log.since(f).iter().map(|e| e.id).collect::<Vec<_>>();
    let mine = Filter {
        session: Some("s-a".into()),
        ..Filter::default()
    };
    assert_eq!(
        ids(&mine),
        [1, 3],
        "its own, and those of no session; not the owner's"
    );
    assert_eq!(
        ids(&Filter {
            after: 1,
            ..mine.clone()
        }),
        [3]
    );
    assert_eq!(
        ids(&Filter {
            changes: vec![61],
            ..Filter::default()
        }),
        [2]
    );
    assert_eq!(
        ids(&Filter {
            owner: true,
            ..Filter::default()
        }),
        [1, 2, 3, 4]
    );
}

#[test]
fn the_log_keeps_the_newest_and_drops_the_old() {
    let mut log = Log::default();
    for i in 0..(KEEP as u64 + 5) {
        log.append("job.ended", Some(1), None, "x", false, 10 + i);
    }
    assert_eq!(log.list.len(), KEEP);
    assert_eq!(log.list.front().map(|e| e.id), Some(6));
    log.append(
        "job.ended",
        Some(1),
        None,
        "late",
        false,
        10 + KEEP_FOR + KEEP as u64 + 10,
    );
    assert_eq!(log.list.len(), 1, "a week later only the newest is kept");
    assert_eq!(log.last, KEEP as u64 + 6, "numbers keep rising");
}

mod of_change {
    use backplane::changes::{Change, Check, Event};
    use backplane::events::of_change;

    use super::step::Apply;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MAIN: &str = "dddddddddddddddddddddddddddddddddddddddd";

    fn kinds(found: &[(String, String)]) -> Vec<&str> {
        found.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn a_change_reports_its_states_and_its_jobs_starts_and_ends() {
        let mut c = Change::propose(60, "x", "feat(x): y", "p", vec!["a/**".into()], "agent");
        assert_eq!(kinds(&of_change(None, &c)), ["change.proposed"]);
        let before = c.clone();
        c.apply(Event::ApprovePlan {
            by: "owner".into(),
            at: 1,
        })
        .expect("plan");
        c.apply(Event::Pushed { sha: A.into() }).expect("pushed");
        assert_eq!(kinds(&of_change(Some(&before), &c)), ["change.pushed"]);
        let before = c.clone();
        c.apply(Event::Bounded {
            sha: A.into(),
            violations: vec![],
            full: vec![],
            checks: vec![Check {
                name: "fmt".into(),
                argv: vec![],
            }],
            tiers: None,
        })
        .expect("bounded");
        c.apply(Event::Started {
            job: "fmt".into(),
            sha: A.into(),
        })
        .expect("started");
        let found = of_change(Some(&before), &c);
        assert_eq!(
            kinds(&found),
            ["change.bounded", "job.passed", "job.started"],
            "the bound job ended, and `fmt` started"
        );
        assert!(found[1].1.contains("`bound`"));
        assert!(found[2].1.contains("`fmt`"));
        let before = c.clone();
        c.apply(Event::Finished {
            job: "fmt".into(),
            sha: A.into(),
            ok: true,
        })
        .expect("finished");
        assert_eq!(
            kinds(&of_change(Some(&before), &c)),
            ["change.checked", "job.passed"]
        );
        let before = c.clone();
        c.apply(Event::Behind { main: MAIN.into() })
            .expect("behind");
        let found = of_change(Some(&before), &c);
        assert_eq!(
            kinds(&found),
            ["change.behind"],
            "its restack is only queued"
        );
        assert_eq!(of_change(Some(&c), &c), [], "nothing new, nothing said");
    }
}
