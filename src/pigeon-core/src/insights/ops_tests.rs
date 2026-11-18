use std::collections::{BTreeSet, VecDeque};

use serde_json::{Value, json};

use super::*;
use crate::change::queue::{Entry, MergeQueue};
use crate::events::Event;

fn ev(id: u64, at: u64, kind: &str, change: u32) -> Event {
    Event {
        id,
        at,
        kind: kind.to_string(),
        change: Some(change),
        session: None,
        summary: String::new(),
        owner: false,
        stage: None,
        model: None,
        subject: None,
        resolves: None,
    }
}

fn entry(number: u32) -> Entry {
    Entry {
        number,
        by: "a".to_string(),
        at: 0,
        docs: false,
    }
}

#[test]
fn percentile_is_nearest_rank() {
    let v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(percentile(&v, 50), Some(5));
    assert_eq!(percentile(&v, 95), Some(10));
    assert_eq!(percentile(&v, 0), Some(1));
    assert_eq!(percentile(&[7], 50), Some(7));
}

#[test]
fn percentile_of_nothing_is_none() {
    assert_eq!(percentile(&[], 50), None);
}

#[test]
fn deploys_keep_the_range_and_the_newest_five() {
    let history: Vec<Value> = (1..=8)
        .map(|i| {
            json!({"sha": format!("s{i}"), "outcome": if i % 2 == 0 { "passed" } else { "failed" },
                   "at": i * 100})
        })
        .collect();
    let d = deploys(&history, 300);
    assert_eq!(d.total, 6);
    assert_eq!(d.passed, 3);
    assert_eq!(d.recent.len(), 5);
    assert_eq!(d.recent[0].sha, "s8");
    assert_eq!(d.recent[4].sha, "s4");
}

#[test]
fn queue_counts_waiting_and_names_the_merging() {
    let q = MergeQueue {
        merging: Some(entry(7)),
        waiting: VecDeque::from([entry(8), entry(9)]),
        ..MergeQueue::default()
    };
    let r = queue(&q, vec![9, 1, 5]);
    assert_eq!(r.depth, 2);
    assert_eq!(r.merging, Some(7));
    assert_eq!(r.waits, vec![9, 1, 5]);
    assert_eq!(r.p50, Some(5));
    assert_eq!(r.max, Some(9));
    let empty = queue(&MergeQueue::default(), vec![]);
    assert_eq!((empty.merging, empty.p50, empty.max), (None, None, None));
}

#[test]
fn a_merge_waits_from_its_latest_approval() {
    let events = [
        ev(1, 0, "change.approved", 1),
        ev(2, 600, "change.approved", 1),
        ev(3, 1_000, "change.approved", 2),
        ev(4, 1_260, "change.merged", 1),
        ev(5, 1_300, "change.merged", 2),
    ];
    assert_eq!(merge_waits(&events), vec![11, 5]);
}

#[test]
fn a_merge_without_an_approval_is_skipped() {
    let events = [
        ev(1, 0, "change.approved", 2),
        ev(2, 600, "change.merged", 1),
    ];
    assert_eq!(merge_waits(&events), Vec::<u32>::new());
}

#[test]
fn only_owner_approvals_count_toward_approval_wait() {
    let events = [
        ev(1, 0, "change.checked", 1),
        ev(2, 0, "change.checked", 2),
        ev(3, 120, "change.approved", 1),
        ev(4, 600, "change.approved", 2),
        ev(5, 700, "change.approved", 3),
    ];
    let owner: BTreeSet<u32> = BTreeSet::from([1, 3]);
    assert_eq!(approval_waits(&events, &owner), vec![2]);
}

#[test]
fn approver_kinds_follow_their_prefix() {
    assert_eq!(approver_kind("window:w1"), "window");
    assert_eq!(approver_kind("mandate:m1"), "mandate");
    assert_eq!(approver_kind("policy:p1"), "policy");
    assert_eq!(approver_kind("owner@example"), "owner");
}

#[test]
fn throughput_zero_fills_each_day() {
    let now = 3 * DAY + 100;
    let rows = [
        json!({"number": 1, "created": 0, "updated": DAY + 5, "approver": "x"}),
        json!({"number": 2, "created": 0, "updated": DAY + 9, "approver": "mandate:m"}),
    ];
    let t = throughput(&rows, DAY + 50, now);
    let days: Vec<(u64, u32)> = t.days.iter().map(|d| (d.day, d.merged)).collect();
    assert_eq!(days, vec![(DAY, 2), (2 * DAY, 0), (3 * DAY, 0)]);
    assert_eq!((t.by.owner, t.by.mandate), (1, 1));
}

#[test]
fn lead_time_is_created_to_merged() {
    let rows = [
        json!({"created": 0, "updated": 600, "approver": "x"}),
        json!({"created": 0, "updated": 1_800, "approver": "x"}),
        json!({"created": 0, "updated": 3_600, "approver": "x"}),
    ];
    assert_eq!(throughput(&rows, 0, 3_600).lead_p50, Some(30));
    assert_eq!(throughput(&[], 0, 10).lead_p50, None);
}

#[test]
fn jobs_merge_ci_and_macos_newest_first() {
    let failed = [json!({"name": "clippy", "change": 4, "status": "failed", "updated": 100})];
    let macos = [
        json!({"kind": "check", "number": 5, "outcome": "lost", "finished_at": 200}),
        json!({"kind": "archive", "number": 6, "outcome": "refused", "finished_at": 50}),
    ];
    let j = jobs(&failed, &macos);
    let at: Vec<u64> = j.iter().map(|r| r.at).collect();
    assert_eq!(at, vec![200, 100, 50]);
    assert_eq!(j[0].name, "check");
    assert_eq!(j[0].change, Some(5));
    assert_eq!(j[1].status, "failed");
    let many: Vec<Value> = (0..12)
        .map(|i| json!({"name": "n", "change": 1, "status": "failed", "updated": i}))
        .collect();
    assert_eq!(jobs(&many, &[]).len(), 10);
}

#[test]
fn a_month_notes_the_seven_day_log() {
    let build = |range| {
        ops(
            range,
            deploys(&[], 0),
            queue(&MergeQueue::default(), vec![]),
            Approvals {
                count: 0,
                p50: None,
                p95: None,
            },
            throughput(&[], 0, 0),
            vec![],
            vec![],
        )
    };
    assert_eq!(
        build(30).note.as_deref(),
        Some("Queue and approval waits cover the last 7 days: the event log keeps no more.")
    );
    assert_eq!(build(7).note, None);
    assert_eq!(build(1).note, None);
}
