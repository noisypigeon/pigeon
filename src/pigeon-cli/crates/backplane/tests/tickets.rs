//! Session tickets; see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §5.

use backplane::sessions::{Class, declare};
use backplane::tickets::{REMEMBER, TTL, Tickets, guarded, holder, require, sign, verify};

const KEY: &[u8] = b"a session key for the tests only";
const A: &str = "https://claude.ai/code/session_a";

#[test]
fn a_ticket_verifies_until_it_expires_and_only_under_its_key() {
    let t = sign(KEY, A, 1, 100);
    assert_eq!(verify(KEY, &t, 99), Ok((A.to_string(), 1)));
    assert!(
        verify(KEY, &t, 100)
            .expect_err("expired")
            .contains("expired")
    );
    assert!(verify(b"another key", &t, 1).is_err());
    let forged = t.replacen(".1.", ".2.", 1);
    assert!(
        verify(KEY, &forged, 1).is_err(),
        "a changed generation breaks the signature"
    );
    assert!(verify(KEY, "nonsense", 1).is_err());
}

#[test]
fn a_label_holds_one_live_ticket_and_replacing_revokes_the_old() {
    let mut t = Tickets::default();
    let first = t.start(A, false, 0).expect("issues");
    assert_eq!((first.generation, first.expires), (1, TTL));
    assert!(t.start(A, false, 1).is_err(), "live already");
    let second = t.start(A, true, 1).expect("replaces");
    assert_eq!(second.generation, 2);
    assert_eq!(
        t.start(A, false, TTL + 2)
            .expect("expired, so anew")
            .generation,
        1
    );
}

#[test]
fn a_ticketed_label_needs_its_current_ticket_and_others_need_none() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    let a = |g| sign(KEY, A, g, TTL);
    let labels = vec![A.to_string()];
    assert_eq!(require(KEY, &t, &labels, Some(&a(1)), 5), Ok(()));
    assert!(
        require(KEY, &t, &labels, None, 5)
            .expect_err("missing")
            .contains("pass it as `ticket`")
    );
    let b = sign(KEY, "session_b", 1, TTL);
    assert!(
        require(KEY, &t, &labels, Some(&b), 5)
            .expect_err("another's")
            .contains("session_b's")
    );
    t.start(A, true, 6).expect("replaced");
    assert!(
        require(KEY, &t, &labels, Some(&a(1)), 7)
            .expect_err("old")
            .contains("replaced")
    );
    assert_eq!(
        require(KEY, &t, &["no-ticket-label".to_string()], None, 7),
        Ok(()),
        "a label without a ticket works as before"
    );
}

#[test]
fn ending_a_ticket_frees_its_label() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    assert!(t.end(A, 2));
    assert!(!t.end(A, 3), "a second end is a no-op");
    assert_eq!(require(KEY, &t, &[A.to_string()], None, 3), Ok(()));
    assert_eq!(t.start(A, false, 3).expect("anew").generation, 1);
}

#[test]
fn a_lapsed_ticket_is_remembered_a_week_to_say_how_long_its_session_has_gone() {
    let mut t = Tickets::default();
    let issued = t.start(A, false, 0).expect("starts");
    assert_eq!(t.gone_for(A, 10), None, "live");
    assert_eq!(t.gone_for(A, issued.expires + 60), Some(60));
    assert!(t.live(A, issued.expires + 60).is_none());
    // A lapsed ticket doesn't block a new start, which begins anew.
    let again = t.start(A, false, issued.expires + 60).expect("restarts");
    assert_eq!(again.generation, 1);
    // Past the week, another label's start forgets it.
    let later = again.expires + REMEMBER;
    t.start("https://claude.ai/code/session_b", false, later)
        .expect("starts");
    assert_eq!(t.gone_for(A, later), None, "forgotten");
}

#[test]
fn an_ended_ticket_is_remembered_from_its_end() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("starts");
    assert!(t.end(A, 100));
    assert!(t.live(A, 101).is_none(), "ended");
    assert_eq!(t.gone_for(A, 7300), Some(7200), "gone since its end");
    assert!(t.start(A, false, 7300).is_ok(), "and it may start again");
}

#[test]
fn a_ticket_keeps_its_class_and_a_replacement_may_declare_another() {
    let mut t = Tickets::default();
    assert_eq!(
        t.start(A, false, 0).expect("issues").declared.model,
        Class::Opus
    );
    let haiku = declare(Some("haiku"), Some("https://claude.ai/code/p"), Some("b3")).expect("ok");
    let second = t.start_as(A, true, 1, haiku).expect("replaces");
    assert_eq!(second.generation, 2);
    let live = t.live(A, 2).expect("live");
    assert_eq!(live.declared.model, Class::Haiku);
    assert_eq!(live.declared.brief.as_deref(), Some("b3"));
}

#[test]
fn a_label_s_class_outlives_its_ticket_and_an_unknown_label_has_none() {
    let mut t = Tickets::default();
    let sonnet = declare(Some("sonnet"), Some("https://claude.ai/code/p"), Some("b1")).expect("ok");
    t.start_as(A, false, 0, sonnet).expect("issues");
    assert!(t.end(A, 5));
    assert_eq!(t.class(A), Some(Class::Sonnet), "lapsed, still known");
    assert_eq!(t.class("https://claude.ai/code/unknown"), None);
}

#[test]
fn a_ticket_stored_before_classes_reads_as_opus() {
    let t: Tickets = serde_json::from_str(
        r#"{"https://claude.ai/code/session_a":{"generation":2,"expires":9}}"#,
    )
    .expect("an older record");
    assert_eq!(t.live(A, 1).expect("live").declared.model, Class::Opus);
}

#[test]
fn a_handoff_needs_the_change_holders_ticket_not_the_recipients() {
    const P: &str = "https://claude.ai/code/session_planner";
    assert_eq!(guarded("change.handoff", Some(P), Some(A)), vec![A]);
    assert_eq!(guarded("change.note", Some(P), Some(A)), vec![P, A]);
    assert_eq!(guarded("task.claim", Some(P), None), vec![P]);
}

const P: &str = "https://claude.ai/code/session_planner";
const Q: &str = "https://claude.ai/code/session_status";

fn handoff(change: u32, to: &str, claimed: Option<&str>) -> backplane::tasks::Task {
    serde_json::from_value(serde_json::json!({
        "id": change, "kind": "handoff", "change": change, "session": to,
        "note": "offered", "created": 0, "claimed_by": claimed,
    }))
    .expect("a task")
}

#[test]
fn an_end_is_clean_only_when_every_held_change_is_offered_to_another_session() {
    use backplane::tickets::clean;
    assert!(clean(A, &[], &[]), "nothing held");
    assert!(clean(A, &[7], &[handoff(7, P, None)]));
    assert!(!clean(A, &[7], &[]), "held with no hand-off");
    assert!(
        !clean(A, &[7, 8], &[handoff(7, P, None)]),
        "one not offered"
    );
    assert!(!clean(A, &[7], &[handoff(7, A, None)]), "offered to itself");
    assert!(
        !clean(A, &[7], &[handoff(7, P, Some(P))]),
        "claimed, so no longer an offer it holds"
    );
}

#[test]
fn only_the_archiver_reports_once_on_a_clean_end_and_any_other_session_when_none() {
    let exec = declare(Some("haiku"), Some(P), Some("b3")).expect("an executor");
    let mut t = Tickets::default();
    t.start(P, false, 0).expect("planner");
    t.start_as(A, false, 0, exec).expect("executor");
    assert!(
        t.record_archive(A, P, "archived", "merged", 1)
            .expect_err("not ended")
            .contains("hasn't ended clean")
    );
    t.end(A, 1);
    let e = t.record_end(A, true, 1).expect("known");
    assert_eq!(e.archiver.as_deref(), Some(P), "an executor's planner");
    assert!(
        t.record_archive(A, A, "archived", "x", 2).is_err(),
        "not itself"
    );
    assert!(
        t.record_archive(A, P, "gone", "x", 2).is_err(),
        "an outcome"
    );
    let a = t
        .record_archive(A, P, "archived", "!9 merged", 2)
        .expect("its archiver");
    assert_eq!((a.by.as_str(), a.outcome.as_str()), (P, "archived"));
    assert!(
        t.record_archive(A, P, "archived", "again", 3).is_err(),
        "once"
    );
    t.start(A, true, 5).expect("anew");
    t.end(A, 6);
    t.record_end(A, true, 6);
    assert!(
        t.record_archive(A, A, "archived", "x", 7).is_err(),
        "not itself, even with no archiver"
    );
    assert_eq!(
        t.record_archive(A, Q, "archived", "by hand", 7)
            .expect("no archiver, so any other session")
            .by,
        Q
    );
}

#[test]
fn an_archiver_reports_without_a_ticket_of_its_own_but_no_other_session_does() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("child");
    t.parented(A, Some(Q.into()));
    t.end(A, 1);
    t.record_end(A, true, 1);
    assert!(
        t.record_archive(A, P, "archived", "x", 2)
            .expect_err("not its archiver")
            .contains("archiver isn't")
    );
    assert!(
        t.record_archive(A, Q, "archived", "by hand", 2).is_ok(),
        "its parent, which never started a ticket"
    );
    assert!(t.unarchived(1 + 2 * 60 * 60).is_empty(), "reported");
}

#[test]
fn a_silent_session_holding_nothing_lapses_after_idle() {
    use backplane::tickets::IDLE;
    use std::collections::{BTreeMap, BTreeSet};
    const B: &str = "https://claude.ai/code/session_b";
    const C: &str = "https://claude.ai/code/session_c";
    let mut t = Tickets::default();
    for l in [A, B, C] {
        t.start(l, false, 100).expect("issues");
    }
    t.start(P, false, 300).expect("issues");
    t.parented(A, Some(P.into()));
    let polled = BTreeMap::from([(A.to_string(), 200), (C.to_string(), 200 + IDLE)]);
    let busy = BTreeSet::from([B.to_string()]);
    assert!(
        t.lapsing(200 + IDLE - 1, &polled, &busy).is_empty(),
        "A polled under IDLE ago"
    );
    assert_eq!(
        t.lapsing(200 + IDLE, &polled, &busy),
        vec![A.to_string()],
        "A is silent past IDLE; P never polled, but started under IDLE ago"
    );
    assert_eq!(
        t.lapsing(300 + IDLE, &polled, &busy),
        vec![A.to_string(), P.to_string()],
        "a session that never polled counts from its ticket's start; B holds work; C polled"
    );
    let e = t.lapse(A, 200 + IDLE).expect("live");
    assert!(e.clean && e.lapsed);
    assert_eq!(e.archiver.as_deref(), Some(P));
    assert!(t.live(A, 200 + IDLE).is_none(), "no longer active");
    assert!(t.lapse(A, 200 + IDLE).is_none(), "once");
    assert!(
        !t.lapsing(200 + IDLE, &polled, &busy)
            .contains(&A.to_string()),
        "an ended ticket isn't named again"
    );
    assert!(
        t.unarchived(200 + IDLE + 2 * 60 * 60).is_empty(),
        "a lapsed end isn't listed as unarchived"
    );
    assert!(
        t.record_archive(A, P, "archived", "archived on Claude", 200 + IDLE)
            .is_ok(),
        "its archiver may still report it"
    );
    t.start(A, false, 300 + IDLE)
        .expect("a lapsed session may start again");
}

#[test]
fn an_unclean_end_is_never_archived_nor_listed() {
    let mut t = Tickets::default();
    t.start(P, false, 0).expect("planner");
    t.start(A, false, 0).expect("child");
    t.parented(A, Some(P.into()));
    t.end(A, 1);
    t.record_end(A, false, 1);
    assert!(t.record_archive(A, P, "archived", "x", 2).is_err());
    assert_eq!(
        t.unarchived(1 + 2 * 60 * 60),
        Vec::new(),
        "unclean, so never listed"
    );
}

#[test]
fn a_clean_end_is_listed_unarchived_from_an_hour_to_a_day() {
    use backplane::tickets::UNARCHIVED;
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    t.end(A, 10);
    t.record_end(A, true, 10);
    assert!(t.unarchived(10 + UNARCHIVED.0 - 1).is_empty(), "too soon");
    assert_eq!(t.unarchived(10 + UNARCHIVED.0), vec![(A.to_string(), None)]);
    assert!(t.unarchived(10 + UNARCHIVED.1).is_empty(), "too late");
}

#[test]
fn a_session_is_archived_by_its_parent_else_its_planner() {
    let mut t = Tickets::default();
    let exec = declare(Some("sonnet"), Some(P), Some("b1")).expect("executor");
    let i = t.start_as(A, false, 0, exec).expect("issues");
    assert_eq!(i.archiver(), Some(P));
    let o = t.start(P, false, 0).expect("issues");
    assert_eq!(o.archiver(), None, "the owner's own sessions name none");
    t.parented(P, Some("https://claude.ai/code/session_s".into()));
    assert_eq!(
        t.live(P, 1).expect("live").archiver(),
        Some("https://claude.ai/code/session_s")
    );
}

#[test]
fn a_holder_is_the_label_of_a_live_current_ticket() {
    let mut t = Tickets::default();
    t.start(A, false, 0).expect("issues");
    let a = |g| sign(KEY, A, g, TTL);
    assert_eq!(holder(KEY, &t, Some(&a(1)), 5), Ok(A.to_string()));
    let missing = "a scratch ref takes its session's ticket (X-Pigeon-Ticket)";
    assert_eq!(holder(KEY, &t, None, 5), Err(missing.into()));
    assert_eq!(holder(KEY, &t, Some("  "), 5), Err(missing.into()));
    assert!(holder(b"another key", &t, Some(&a(1)), 5).is_err());
    t.start(A, true, 6).expect("replaces");
    assert_eq!(
        holder(KEY, &t, Some(&a(1)), 7),
        Err(format!(
            "this ticket was replaced; session {A}'s newer one is needed"
        ))
    );
    assert_eq!(holder(KEY, &t, Some(&a(2)), 7), Ok(A.to_string()));
    t.end(A, 8);
    assert_eq!(
        holder(KEY, &t, Some(&a(2)), 9),
        Err(format!("session {A}'s ticket has ended"))
    );
}
