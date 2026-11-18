//! The feed's items, needs, readers and open set; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §3.

use super::*;
use crate::change::stage::{Needs as StageNeeds, Stage};
use crate::events::Event;

fn stage(needs: StageNeeds) -> Stage {
    Stage {
        stage: "checking".to_string(),
        needs,
        waits_on: None,
        next: "wait".to_string(),
        since: None,
    }
}

fn ev(id: u64, kind: &str, change: Option<u32>, st: Option<StageNeeds>) -> Event {
    Event {
        id,
        at: id,
        kind: kind.to_string(),
        change,
        session: None,
        summary: format!("{kind} {id}"),
        owner: false,
        stage: st.map(stage),
        model: None,
        subject: None,
        resolves: None,
    }
}

fn it(id: u64, t: ItemType, n: Needs, change: Option<u32>) -> Item {
    Item {
        id,
        at: id,
        item_type: t,
        kind: t.name().to_string(),
        needs: n,
        change,
        session: None,
        summary: String::new(),
        owner: false,
        resolves: None,
    }
}

#[test]
fn names_parse_back() {
    for t in ItemType::ALL {
        assert_eq!(ItemType::parse(t.name()), Some(t));
    }
    for n in [Needs::Owner, Needs::Agent, Needs::None] {
        assert_eq!(Needs::parse(n.name()), Some(n));
    }
    let names: std::collections::BTreeSet<_> = ItemType::ALL.iter().map(|t| t.name()).collect();
    assert_eq!(ItemType::ALL.len(), 13);
    assert_eq!(names.len(), 13);
    assert_eq!(ItemType::parse("nope"), None);
}

#[test]
fn kinds_map_to_types() {
    let cases = [
        ("approval.pending", ItemType::ApprovalPending),
        ("grant.requested", ItemType::GrantPending),
        ("owner.asked", ItemType::OwnerAsk),
        ("owner.answered", ItemType::OwnerAnswered),
        ("task.made", ItemType::TaskWaiting),
        ("deploy.failed", ItemType::DeployFailed),
        ("deploy.passed", ItemType::DeployPassed),
        ("pool.paused", ItemType::PoolPaused),
        ("digest", ItemType::Digest),
        ("session.ended", ItemType::SessionEnded),
        ("session.archivable", ItemType::SessionEnded),
        ("provider.refused", ItemType::ProviderRefused),
        ("change.checked", ItemType::ChangeStage),
        ("merge.released", ItemType::ChangeStage),
        ("job.passed", ItemType::Note),
        ("auto.paused", ItemType::Note),
    ];
    for (k, t) in cases {
        assert_eq!(item_type(k), t, "{k}");
    }
}

#[test]
fn needs_follow_stage_and_wakes() {
    let n = |e: &Event| item(e).needs;
    assert_eq!(n(&ev(1, "approval.pending", Some(1), None)), Needs::Owner);
    assert_eq!(
        n(&ev(2, "change.checked", Some(1), Some(StageNeeds::Agent))),
        Needs::Agent
    );
    assert_eq!(
        n(&ev(3, "change.checked", Some(1), Some(StageNeeds::Owner))),
        Needs::Owner
    );
    assert_eq!(
        n(&ev(
            4,
            "change.checked",
            Some(1),
            Some(StageNeeds::Provider)
        )),
        Needs::None
    );
    assert_eq!(
        n(&ev(5, "job.passed", Some(1), Some(StageNeeds::Agent))),
        Needs::None
    );
    assert_eq!(n(&ev(6, "auto.paused", None, None)), Needs::Agent);
    assert_eq!(n(&ev(7, "deploy.passed", None, None)), Needs::None);
}

#[test]
fn item_serializes_without_unset_fields() {
    let i = item(&ev(1, "approval.pending", None, None));
    let v = serde_json::to_value(&i).unwrap();
    assert_eq!(v["type"], "approval.pending");
    for k in ["resolves", "owner", "change", "session"] {
        assert!(v.get(k).is_none(), "{k}");
    }
    let back: Item = serde_json::from_value(v).unwrap();
    assert_eq!(back, i);
}

#[test]
fn readers_see_what_they_may() {
    let mut owner_only = it(1, ItemType::OwnerAsk, Needs::Owner, None);
    owner_only.owner = true;
    assert!(visible(&owner_only, &Reader::Owner));
    assert!(!visible(&owner_only, &Reader::Session("a".into())));

    let me = Reader::Session("a".into());
    let mut mine = it(2, ItemType::ChangeStage, Needs::Agent, Some(1));
    mine.session = Some("a".into());
    let mut theirs = mine.clone();
    theirs.session = Some("b".into());
    let deploy = it(3, ItemType::DeployFailed, Needs::Owner, None);
    let grant = it(4, ItemType::GrantPending, Needs::Owner, None);
    assert!(visible(&mine, &me));
    assert!(visible(&deploy, &me));
    assert!(!visible(&theirs, &me));
    assert!(!visible(&grant, &me));

    assert!(visible(&deploy, &Reader::Member));
    assert!(!visible(&mine, &Reader::Member));
    assert!(!visible(&owner_only, &Reader::Member));
}

#[test]
fn open_set_closes_superseded_items() {
    let pending = it(1, ItemType::ApprovalPending, Needs::Owner, Some(7));
    let staged = it(2, ItemType::ChangeStage, Needs::Agent, Some(7));
    let items = [pending, staged];
    let ids: Vec<u64> = open(&items).iter().map(|i| i.id).collect();
    assert_eq!(ids, [2]);

    let note = it(3, ItemType::Note, Needs::None, None);
    assert_eq!(open(&[note]).len(), 0);

    let ask = it(4, ItemType::OwnerAsk, Needs::Owner, None);
    let mut answer = it(5, ItemType::OwnerAnswered, Needs::Agent, None);
    answer.resolves = Some(4);
    let items = [ask, answer];
    let ids: Vec<u64> = open(&items).iter().map(|i| i.id).collect();
    assert_eq!(ids, [5]);
}

#[test]
fn read_filters() {
    let events: Vec<Event> = (1..=5).map(|i| ev(i, "task.made", None, None)).collect();
    let q = |f: &dyn Fn(&mut Query)| {
        let mut q = Query::default();
        f(&mut q);
        read(&events, &Reader::Owner, &q)
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(q(&|q| q.after = 3), [4, 5]);
    assert_eq!(q(&|q| q.needs = Some(Needs::Agent)), [1, 2, 3, 4, 5]);
    assert_eq!(q(&|q| q.needs = Some(Needs::Owner)).len(), 0);
    assert_eq!(q(&|q| q.limit = 2), [1, 2]);

    let many: Vec<Event> = (1..=(crate::events::PAGE as u64 + 10))
        .map(|i| ev(i, "task.made", None, None))
        .collect();
    let got = read(&many, &Reader::Owner, &Query::default());
    assert_eq!(got.len(), crate::events::PAGE);

    // An item before `after` is still closed by one after it.
    let log = vec![
        ev(1, "approval.pending", Some(9), None),
        ev(2, "change.approved", Some(9), Some(StageNeeds::Agent)),
    ];
    let got = read(
        &log,
        &Reader::Owner,
        &Query {
            open: true,
            after: 0,
            ..Query::default()
        },
    );
    assert_eq!(got.iter().map(|i| i.id).collect::<Vec<_>>(), [2]);
    let got = read(
        &log,
        &Reader::Owner,
        &Query {
            open: true,
            after: 1,
            ..Query::default()
        },
    );
    assert_eq!(got.iter().map(|i| i.id).collect::<Vec<_>>(), [2]);
}

#[test]
fn reader_is_owner_then_session_then_member() {
    assert_eq!(reader(true, Some("a")), Reader::Owner);
    assert_eq!(reader(false, Some("a")), Reader::Session("a".to_string()));
    assert_eq!(reader(false, Some("  ")), Reader::Member);
    assert_eq!(reader(false, None), Reader::Member);
}

#[test]
fn needs_parse_from_text() {
    assert_eq!(parse_needs(Some("owner")), Some(Needs::Owner));
    assert_eq!(parse_needs(Some("agent")), Some(Needs::Agent));
    assert_eq!(parse_needs(Some("none")), Some(Needs::None));
    assert_eq!(parse_needs(Some(" ")), None);
    assert_eq!(parse_needs(None), None);
    assert_eq!(parse_needs(Some("x")), None);
}

#[test]
fn item_carries_resolves() {
    let mut e = ev(5, "owner.answered", None, None);
    e.resolves = Some(3);
    assert_eq!(item(&e).resolves, Some(3));
    let ask = it(3, ItemType::OwnerAsk, Needs::Owner, None);
    let answer = item(&e);
    assert!(open(&[ask, answer]).iter().all(|i| i.id != 3));
}
