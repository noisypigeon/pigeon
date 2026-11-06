//! Grants: one use, a deadline, the head they were made for, the operation they name.

use backplane::changes::{Change, State};
use backplane::grants::{self, Grants, TTL};

const HEAD: &str = "1111111111111111111111111111111111111111";
const NEXT: &str = "2222222222222222222222222222222222222222";

fn change(state: State, head: Option<&str>) -> Change {
    let mut c = Change::propose(
        7,
        "approvals",
        "feat: x",
        "plan",
        vec!["**".into()],
        "agent",
    );
    c.state = state;
    c.head = head.map(str::to_string);
    c
}

#[test]
fn a_grant_is_redeemed_once_for_its_operation_and_head() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    let minted = g.mint(&c, "change.approve", "tok", 100).expect("mint");
    assert_eq!(minted.head.as_deref(), Some(HEAD));
    assert_eq!(minted.expires, 100 + TTL);
    assert!(!g.0.contains_key("tok"), "stored by hash, not the token");
    assert_eq!(g.peek("tok", 7, 200).expect("peek"), &minted);
    assert_eq!(g.redeem("tok", "change.approve", &c, 200), Ok(minted));
    assert!(
        g.redeem("tok", "change.approve", &c, 201).is_err(),
        "single use"
    );
    assert!(g.peek("tok", 7, 201).is_err());
}

#[test]
fn an_expired_grant_is_refused_and_pruned() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    g.mint(&c, "change.approve", "old", 0).expect("mint");
    let why = g
        .redeem("old", "change.approve", &c, TTL)
        .expect_err("expired");
    assert!(why.contains("expired"), "{why}");
    g.mint(&c, "change.approve", "a", 0).expect("mint");
    g.mint(&c, "change.close", "b", TTL + 1).expect("mint");
    assert_eq!(g.0.len(), 1, "minting drops expired grants");
}

#[test]
fn a_grant_is_void_once_the_head_moves() {
    let mut c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    g.mint(&c, "change.approve", "tok", 0).expect("mint");
    c.head = Some(NEXT.into());
    let why = g.redeem("tok", "change.approve", &c, 1).expect_err("moved");
    assert!(why.contains("moved"), "{why}");
    assert!(
        g.redeem("tok", "change.approve", &c, 1).is_err(),
        "spent anyway"
    );
}

#[test]
fn a_grant_is_only_for_its_operation_and_change() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    g.mint(&c, "change.approve", "tok", 0).expect("mint");
    assert!(g.peek("tok", 8, 1).is_err(), "another change");
    assert!(g.peek("nope", 7, 1).is_err(), "another token");
    let why = g
        .redeem("tok", "change.merge", &c, 1)
        .expect_err("wrong op");
    assert!(why.contains("change.approve"), "{why}");
}

#[test]
fn a_grant_is_offered_only_where_its_operation_applies() {
    let ok = |s, head, op| grants::offerable(&change(s, head), op).is_ok();
    assert!(ok(State::Proposed, None, "change.authorize"));
    assert!(!ok(State::Planned, None, "change.authorize"));
    assert!(ok(State::Checked, Some(HEAD), "change.approve"));
    assert!(!ok(State::Pushed, Some(HEAD), "change.approve"));
    assert!(!ok(State::Failed, Some(HEAD), "change.approve"));
    assert!(ok(State::Approved, Some(HEAD), "change.merge"));
    assert!(!ok(State::Checked, Some(HEAD), "change.merge"));
    assert!(ok(State::Failed, Some(HEAD), "change.close"));
    assert!(!ok(State::Merged, Some(HEAD), "change.close"));
    assert!(!ok(State::Checked, Some(HEAD), "change.propose"));
    let mut g = Grants::default();
    assert!(
        g.mint(&change(State::Pushed, Some(HEAD)), "change.approve", "t", 0)
            .is_err()
    );
    assert!(g.0.is_empty());
}
