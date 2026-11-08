//! The event log; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §1.

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
