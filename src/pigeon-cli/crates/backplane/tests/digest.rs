//! The owner's digest; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §4.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

use backplane::approve::Waiting;
use backplane::mandates::{Mandates, State};
use backplane::tasks::{DigestInput, STALE, Tasks, digest};

const ADR: &str = ".claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md";

#[test]
fn an_empty_digest_asks_nothing_of_the_owner() {
    let d = digest(&DigestInput::default(), 100);
    assert!(d.is_empty());
    assert!(!d.needs_owner());
    assert_eq!(d.lines(), Vec::<String>::new());
}

#[test]
fn the_digest_names_what_waits_on_the_owner_and_what_merged_without_them() {
    let waiting = [
        Waiting {
            number: 62,
            operation: "change.merge".into(),
            title: "t".into(),
            path: "/approve/62?grant=x".into(),
            expires: STALE + 900,
        },
        Waiting {
            number: 61,
            operation: "change.approve".into(),
            title: "t".into(),
            path: "/approve/61?grant=y".into(),
            expires: STALE + 500,
        },
        Waiting {
            number: 60,
            operation: "change.merge".into(),
            title: "t".into(),
            path: "/approve/60?grant=z".into(),
            expires: 50,
        },
    ];
    let mut tasks = Tasks::default();
    tasks.add(
        "reconcile",
        70,
        Some(60),
        Some("s-a".into()),
        "fix: reconcile !60",
        0,
    );
    tasks.add("review", 71, None, None, "fresh", STALE);
    let mut mandates = Mandates::default();
    kept!(mandates.propose(ADR, "task", vec!["src/**".into()], false, None, None))
        .expect("proposes");
    kept!(mandates.offer(1, "tok", 0, 100)).expect("offers");
    kept!(mandates.grant("tok", "owner", 1)).expect("grants");
    kept!(mandates.pause_all("the deploy of abc failed"));
    let failed = [("abcdefabcdefabcdef".to_string(), "health".to_string())];
    let merged = [(59, "mandate:1".to_string())];
    let d = digest(
        &DigestInput {
            waiting: &waiting,
            tasks: Some(&tasks),
            mandates: &mandates.list,
            failed_deploys: &failed,
            merged_without_owner: &merged,
            mirror: None,
            macos: &[],
        },
        STALE + 10,
    );
    assert!(d.needs_owner());
    assert_eq!(
        d.approvals.iter().map(|a| a.0).collect::<Vec<_>>(),
        [61, 62],
        "live ones, soonest to expire first"
    );
    assert_eq!(d.stale_tasks.len(), 1, "only the task unclaimed two hours");
    assert_eq!(d.paused, [(1, "the deploy of abc failed".to_string())]);
    assert_eq!(mandates.list[0].state, State::Paused);
    let lines = d.lines();
    assert!(lines[0].contains("`change.approve` on change 61"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("unclaimed: task 1 (reconcile)"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l == "deploy failed: abcdefabcdef: health")
    );
    assert!(
        lines
            .iter()
            .any(|l| l == "merged without you: change 59, by mandate:1")
    );
}

#[test]
fn only_merges_without_the_owner_are_news_but_not_a_push() {
    let merged = [(59, "mandate:1".to_string())];
    let d = digest(
        &DigestInput {
            merged_without_owner: &merged,
            ..DigestInput::default()
        },
        0,
    );
    assert!(!d.is_empty());
    assert!(!d.needs_owner());
}

#[test]
fn macos_moves_are_news_but_not_a_push() {
    let lines = ["!300 ios moved: github gave no machine".to_string()];
    let d = digest(
        &DigestInput {
            macos: &lines,
            ..DigestInput::default()
        },
        0,
    );
    assert!(!d.is_empty() && !d.needs_owner());
    assert_eq!(d.lines(), ["macOS: !300 ios moved: github gave no machine"]);
}

#[test]
fn a_mirror_behind_needs_the_owner() {
    let d = digest(
        &DigestInput {
            mirror: Some("mirror: GitHub has been behind for 25 hours"),
            ..DigestInput::default()
        },
        0,
    );
    assert!(d.needs_owner());
    assert_eq!(d.lines(), ["mirror: GitHub has been behind for 25 hours"]);
}

#[test]
fn sessions_are_shown_but_never_pushed() {
    use backplane::tasks::{Digest, SessionLine};
    let d = Digest {
        sessions: vec![SessionLine {
            label: "https://claude.ai/code/session_abc".into(),
            listening: false,
            changes: vec![
                (5, "checked".into(), "owner".into()),
                (9, "failed".into(), "agent".into()),
            ],
            model: None,
        }],
        ..Digest::default()
    };
    assert_eq!(
        d.lines(),
        ["session session_abc: not listening; !9 failed (agent), !5 checked (owner)"],
        "the changes needing the agent first"
    );
    assert_eq!(d.owner_lines(), Vec::<String>::new());
    assert!(!d.needs_owner(), "a session alone pushes nothing");
}

#[test]
fn a_change_needing_a_session_that_isnt_listening_is_pushed_as_stalled() {
    // !364 waited on its unattended session, and !374 on it; see the session-stages ADR's
    // 2026-11-13 amendment on stalls.
    use backplane::tasks::{STALL, Stall, stalled};
    assert_eq!(stalled(Some(0), None, "agent", STALL - 1), None, "not yet");
    assert_eq!(stalled(Some(0), None, "agent", STALL), Some(Some(STALL)));
    assert_eq!(stalled(None, None, "agent", 0), Some(None), "never polled");
    assert_eq!(
        stalled(None, None, "owner", 0),
        None,
        "the owner's turn is no stall"
    );
    let mut d = digest(&DigestInput::default(), 100);
    assert!(!d.needs_owner());
    d.stalled.push(Stall {
        number: 364,
        session: "https://claude.ai/code/session_x".into(),
        stage: "approved".into(),
        next: "ask to merge (`change_merge`)".into(),
        quiet: Some(3 * 3600 + 5 * 60),
        waiting: vec![374],
        model: None,
    });
    assert!(d.needs_owner(), "a stall is pushed");
    assert_eq!(
        d.owner_lines(),
        [
            "stalled: !364 approved needs session session_x (quiet 3h05m): ask to merge \
          (`change_merge`); !374 waits on it; start a session to take it over, or close it"
        ]
    );
}

#[test]
fn a_sessions_class_is_named_on_its_lines() {
    // See the model-tiered briefs ADR §7.
    use backplane::sessions::Class;
    use backplane::tasks::{Digest, SessionLine, Stall};
    let d = Digest {
        sessions: vec![SessionLine {
            label: "https://claude.ai/code/session_h".into(),
            listening: true,
            changes: vec![(7, "in-review".into(), "agent".into())],
            model: Some(Class::Haiku),
        }],
        stalled: vec![Stall {
            number: 8,
            session: "https://claude.ai/code/session_s".into(),
            stage: "failed".into(),
            next: "fix it".into(),
            quiet: None,
            waiting: Vec::new(),
            model: Some(Class::Sonnet),
        }],
        ..Digest::default()
    };
    let lines = d.lines();
    assert!(
        lines.contains(&"session session_h (haiku): listening; !7 in-review (agent)".to_string()),
        "{lines:?}"
    );
    assert_eq!(
        d.owner_lines(),
        [
            "stalled: !8 failed needs session session_s (sonnet) (never polled): fix it; start \
             a session to take it over, or close it"
        ]
    );
}

#[test]
fn the_digest_names_a_mandate_held_by_a_failed_deploy() {
    let mut mandates = Mandates::default();
    kept!(mandates.propose(ADR, "task", vec!["src/**".into()], false, None, None))
        .expect("proposes");
    kept!(mandates.offer(1, "tok", 0, 100)).expect("offers");
    kept!(mandates.grant("tok", "owner", 1)).expect("grants");
    let (held, ids, _) = mandates.hold_all("abcdefabcdefabcdef", "token", 60);
    assert_eq!(ids, [1]);
    let d = digest(
        &DigestInput {
            mandates: &held.list,
            ..DigestInput::default()
        },
        660,
    );
    assert!(d.needs_owner());
    assert_eq!(
        d.held,
        [
            "held: mandate 1 for 10 min, 1 failed deploy: merges once a deploy of main passes: \
          the deploy of abcdefabcdef failed: token; resumes when a deploy passes"
        ]
    );
    assert_eq!(held.list[0].state, State::Held);
}

#[test]
fn failed_starts_and_lost_containers_are_counted_in_a_line_that_is_never_pushed_alone() {
    let event = |id: u64, kind: &str, summary: &str| backplane::events::Event {
        id,
        at: id,
        kind: kind.into(),
        change: Some(461),
        session: None,
        summary: summary.into(),
        owner: false,
        stage: None,
        model: None,
    };
    let events = [
        event(
            1,
            "job.retrying",
            "!461 job `merge` didn't start (1 of 8): 503",
        ),
        event(2, "job.started", "!461 job `merge`"),
        event(
            3,
            "job.retrying",
            "!464 job `restack` didn't start (1 of 8): 502",
        ),
    ];
    let line = backplane::tasks::containers(&events).expect("a line");
    assert!(line.contains("2 failed start(s)"), "{line}");
    assert!(
        line.ends_with("!464 job `restack` didn't start (1 of 8): 502"),
        "{line}"
    );
    assert_eq!(backplane::tasks::containers(&events[1..2]), None);
    let mut d = digest(&DigestInput::default(), 100);
    d.containers = Some(line.clone());
    assert!(!d.needs_owner(), "never pushed alone");
    assert!(!d.is_empty());
    assert_eq!(d.lines(), vec![line]);
}

// See `.claude/decisions/2026-11-15-development-sessions-stop-only-for-oversight.md` §5.
#[test]
fn the_digest_counts_stops_by_kind_and_shows_none_without_them() {
    use backplane::events::Event;
    let e = |id: u64, kind: &str, summary: &str| Event {
        id,
        at: id,
        kind: kind.into(),
        change: Some(1),
        session: None,
        summary: summary.into(),
        owner: false,
        stage: None,
        model: None,
    };
    assert_eq!(backplane::tasks::stops(&[]), None);
    assert_eq!(
        backplane::tasks::stops(&[e(1, "change.checked", "x")]),
        None
    );
    let events = [
        e(
            1,
            "approval.pending",
            "`change.approve` on !1 waits for the owner",
        ),
        e(
            2,
            "approval.pending",
            "`change.approve` on !2 waits for the owner",
        ),
        e(
            3,
            "approval.pending",
            "`change.authorize` on !3 waits for the owner",
        ),
        e(4, "grant.requested", "grant observability asked"),
        e(5, "change.nudged", "!4 has needed its session for 1h"),
        e(6, "task.made", "task (escalation) on !5: wait?"),
        e(7, "task.made", "task (review) on !6: x"),
    ];
    assert_eq!(
        backplane::tasks::stops(&events).as_deref(),
        Some(
            "stops since last digest: owner 4 (approve 2, authorize 1, grant 1); agent nudged 1; escalated 1"
        )
    );
}

#[test]
fn the_digest_shows_archives_and_clean_ends_not_archived_without_pushing() {
    let lines = backplane::tasks::archive_lines(
        &["session_a archived by session_p: !9 merged".into()],
        &[("https://claude.ai/code/session_b".into(), None)],
    );
    assert_eq!(
        lines,
        vec![
            "session: session_a archived by session_p: !9 merged".to_string(),
            "ended clean, not archived: session_b (archiver none)".to_string(),
        ]
    );
    let d = backplane::tasks::Digest {
        archives: lines,
        ..backplane::tasks::Digest::default()
    };
    assert!(!d.needs_owner(), "shown, never pushed");
    assert_eq!(d.lines().len(), 2);
}

// !542, !543 and !544 were named stalled minutes after their plans were authorized, while their
// sessions wrote the code; see
// `.claude/decisions/2026-11-15-development-sessions-stop-only-for-oversight.md`'s 2026-11-15
// amendment on stalls.
#[test]
fn a_change_authorized_minutes_ago_is_not_stalled_whatever_its_sessions_last_poll() {
    use backplane::tasks::{STALL, stalled};
    let now = 100_000;
    let authorized = now - 5 * 60;
    assert_eq!(
        stalled(None, Some(authorized), "agent", now),
        None,
        "never polled"
    );
    let slept = now - (8 * 3600 + 15 * 60);
    assert_eq!(
        stalled(Some(slept), Some(authorized), "agent", now),
        None,
        "an old poll, a fresh move"
    );
    assert_eq!(
        stalled(None, Some(now - STALL), "agent", now),
        Some(Some(STALL)),
        "a grace of STALL from the move"
    );
    assert_eq!(
        stalled(Some(now - 60), Some(now - 2 * STALL), "agent", now),
        None,
        "a recent poll"
    );
}
