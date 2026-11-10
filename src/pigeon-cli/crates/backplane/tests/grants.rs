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
    let used = g.redeem("tok", "change.approve", &c, 200).expect("redeems");
    assert_eq!(used.used, Some(200));
    assert_eq!(backplane::grants::Grant { used: None, ..used }, minted);
    let again = g
        .redeem("tok", "change.approve", &c, 201)
        .expect_err("single use");
    assert_eq!(
        again,
        "this link was already used: `change.approve` on change 7 is done"
    );
    assert!(
        g.peek("tok", 7, 201)
            .expect_err("used")
            .contains("already used")
    );
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

#[test]
fn a_page_grant_lives_a_day_and_a_card_grant_fifteen_minutes() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    let page = g
        .mint_for(&c, "change.approve", "page", 100, grants::PAGE_TTL)
        .expect("mint");
    assert_eq!(page.expires, 100 + 24 * 60 * 60);
    let card = g.mint(&c, "change.approve", "card", 100).expect("mint");
    assert_eq!(card.expires, 100 + TTL);
    assert!(
        g.peek("page", 7, 100 + TTL + 1).is_ok(),
        "the page's link outlives the card's"
    );
}

#[test]
fn a_grant_still_applies_only_while_its_operation_and_head_do() {
    let mut c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    let grant = g.mint(&c, "change.approve", "tok", 0).expect("mint");
    assert!(backplane::grants::still_applies(&c, &grant));
    c.state = State::Merged;
    assert!(
        !backplane::grants::still_applies(&c, &grant),
        "merged since"
    );
    c.state = State::Checked;
    c.head = Some(NEXT.into());
    assert!(!backplane::grants::still_applies(&c, &grant), "a new head");
}

#[test]
fn the_break_glass_is_offered_only_on_an_open_head_with_a_code_check_not_passed() {
    use backplane::changes::{Job, JobStatus};
    use backplane::control::BREAK_GLASS;
    let job = |name: &str, cmd: &str, status: JobStatus| Job {
        name: name.into(),
        argv: cmd.split(' ').map(str::to_string).collect(),
        status,
        attempts: 1,
    };
    let mut c = change(State::Bounded, Some(HEAD));
    c.jobs = vec![
        job("policy.check", "cargo run -p pigeon", JobStatus::Queued),
        job("fmt", "cargo fmt --all", JobStatus::Passed),
    ];
    assert!(
        grants::offerable(&c, BREAK_GLASS).is_err(),
        "no code check left"
    );
    c.jobs
        .push(job("test", "cargo test --workspace", JobStatus::Running));
    assert!(grants::offerable(&c, BREAK_GLASS).is_ok());
    c.state = State::Merged;
    assert!(grants::offerable(&c, BREAK_GLASS).is_err());
    c.state = State::Failed;
    let mut all = Grants::default();
    let g = all.mint(&c, BREAK_GLASS, "tok", 10).expect("mints");
    assert!(g.needs_code(), "the owner proves it each time");
    assert!(grants::OPERATIONS.contains(&BREAK_GLASS));
}
