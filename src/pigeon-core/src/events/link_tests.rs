//! The log's subjects and links; see the ADR's 2026-11-17 amendment on step 8.

use super::*;

fn add(log: &mut Log, kind: &str, subject: &str) -> u64 {
    let id = log.append(kind, None, None, kind, false, 1);
    log.link(id, subject);
    id
}

fn get(log: &Log, id: u64) -> &Event {
    log.list.iter().find(|e| e.id == id).unwrap()
}

#[test]
fn answer_links_to_ask() {
    let mut log = Log::default();
    let ask = add(&mut log, "owner.asked", "owner:7");
    let ans = add(&mut log, "owner.answered", "owner:7");
    assert_eq!(get(&log, ans).resolves, Some(ask));
    assert_eq!(get(&log, ask).resolves, None);
}

#[test]
fn doubled_openers_close_newest() {
    let mut log = Log::default();
    let a = add(&mut log, "approval.pending", "change:1");
    let b = add(&mut log, "approval.pending", "change:1");
    assert_eq!(get(&log, a).resolves, None);
    assert_eq!(get(&log, b).resolves, None);
    let g = add(&mut log, "grant.granted", "change:1");
    assert_eq!(get(&log, g).resolves, Some(b));
}

#[test]
fn closer_without_opener_stays_unresolved() {
    let mut log = Log::default();
    let ans = add(&mut log, "owner.answered", "owner:1");
    assert_eq!(get(&log, ans).resolves, None);
}

#[test]
fn opener_closes_once() {
    let mut log = Log::default();
    let ask = add(&mut log, "owner.asked", "owner:7");
    let first = add(&mut log, "owner.answered", "owner:7");
    let second = add(&mut log, "owner.answered", "owner:7");
    assert_eq!(get(&log, first).resolves, Some(ask));
    assert_eq!(get(&log, second).resolves, None);
}

#[test]
fn other_subjects_do_not_link() {
    let mut log = Log::default();
    add(&mut log, "owner.asked", "owner:7");
    let ans = add(&mut log, "owner.answered", "owner:8");
    assert_eq!(get(&log, ans).resolves, None);
}

#[test]
fn unknown_id_or_blank_subject_changes_nothing() {
    let mut log = Log::default();
    let id = log.append("owner.asked", None, None, "x", false, 1);
    let before = log.clone();
    log.link(99, "owner:7");
    assert_eq!(log, before);
    log.link(id, "  ");
    assert_eq!(log, before);
}

#[test]
fn subject_and_resolves_round_trip() {
    let mut log = Log::default();
    let id = log.append("owner.asked", None, None, "x", false, 1);
    let plain = serde_json::to_string(get(&log, id)).unwrap();
    assert!(!plain.contains("subject") && !plain.contains("resolves"));
    let mut e = get(&log, id).clone();
    e.subject = Some("owner:7".into());
    e.resolves = Some(3);
    let back: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(back, e);
}
