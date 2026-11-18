//! Grants: one use, a deadline, the head they were made for, the operation they name.

use agent::core::approval::grants::{self, Grants, TTL};
use agent::core::change::{Change, State};

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
    let minted = kept!(g.mint(&c, "change.approve", "tok", 100)).expect("mint");
    assert_eq!(minted.head.as_deref(), Some(HEAD));
    assert_eq!(minted.expires, 100 + TTL);
    assert!(!g.0.contains_key("tok"), "stored by hash, not the token");
    assert_eq!(g.peek("tok", 7, 200).expect("peek"), &minted);
    let used = kept!(g.redeem("tok", "change.approve", &c, 200)).expect("redeems");
    assert_eq!(used.used, Some(200));
    assert_eq!(
        agent::core::approval::grants::Grant { used: None, ..used },
        minted
    );
    let again = kept!(g.redeem("tok", "change.approve", &c, 201)).expect_err("single use");
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
    kept!(g.mint(&c, "change.approve", "old", 0)).expect("mint");
    let why = kept!(g.redeem("old", "change.approve", &c, TTL)).expect_err("expired");
    assert!(why.contains("expired"), "{why}");
    kept!(g.mint(&c, "change.approve", "a", 0)).expect("mint");
    kept!(g.mint(&c, "change.close", "b", TTL + 1)).expect("mint");
    assert_eq!(g.0.len(), 1, "minting drops expired grants");
}

#[test]
fn a_grant_is_void_once_the_head_moves() {
    let mut c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    kept!(g.mint(&c, "change.approve", "tok", 0)).expect("mint");
    c.head = Some(NEXT.into());
    let why = kept!(g.redeem("tok", "change.approve", &c, 1)).expect_err("moved");
    assert!(why.contains("moved"), "{why}");
    assert!(
        kept!(g.redeem("tok", "change.approve", &c, 1)).is_err(),
        "spent anyway"
    );
}

#[test]
fn a_grant_is_only_for_its_operation_and_change() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    kept!(g.mint(&c, "change.approve", "tok", 0)).expect("mint");
    assert!(g.peek("tok", 8, 1).is_err(), "another change");
    assert!(g.peek("nope", 7, 1).is_err(), "another token");
    let why = kept!(g.redeem("tok", "change.merge", &c, 1)).expect_err("wrong op");
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
    assert!(kept!(g.mint(&change(State::Pushed, Some(HEAD)), "change.approve", "t", 0)).is_err());
    assert!(g.0.is_empty());
}

#[test]
fn a_page_grant_lives_a_day_and_a_card_grant_fifteen_minutes() {
    let c = change(State::Checked, Some(HEAD));
    let mut g = Grants::default();
    let page =
        kept!(g.mint_for(&c, "change.approve", "page", 100, grants::PAGE_TTL)).expect("mint");
    assert_eq!(page.expires, 100 + 24 * 60 * 60);
    let card = kept!(g.mint(&c, "change.approve", "card", 100)).expect("mint");
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
    let grant = kept!(g.mint(&c, "change.approve", "tok", 0)).expect("mint");
    assert!(agent::core::approval::grants::still_applies(&c, &grant));
    c.state = State::Merged;
    assert!(
        !agent::core::approval::grants::still_applies(&c, &grant),
        "merged since"
    );
    c.state = State::Checked;
    c.head = Some(NEXT.into());
    assert!(
        !agent::core::approval::grants::still_applies(&c, &grant),
        "a new head"
    );
}

#[test]
fn the_break_glass_is_offered_only_on_an_open_head_with_a_code_check_not_passed() {
    use agent::control::BREAK_GLASS;
    use agent::core::change::{Job, JobStatus};
    let job = |name: &str, cmd: &str, status: JobStatus| Job {
        name: name.into(),
        argv: cmd.split(' ').map(str::to_string).collect(),
        status,
        attempts: 1,
        start_failures: 0,
        last_error: None,
        lane: None,
    };
    let mut c = change(State::Bounded, Some(HEAD));
    c.jobs = vec![
        job(
            "policy.check",
            "cargo run -p pigeon-harness",
            JobStatus::Queued,
        ),
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
    let g = kept!(all.mint(&c, BREAK_GLASS, "tok", 10)).expect("mints");
    assert!(g.needs_code(), "the owner proves it each time");
    assert!(grants::OPERATIONS.contains(&BREAK_GLASS));
}

/// Hands out `op`'s page grant on `c` at `now`, keeping the next grants and pages.
fn page(
    g: &mut Grants,
    pages: &mut grants::Pages,
    c: &Change,
    op: &str,
    fresh: &str,
    now: u64,
) -> grants::Page {
    let (next, open, out) = g.page(pages, c, op, fresh, now);
    *g = next;
    *pages = open;
    out.expect("a page grant")
}

#[test]
fn a_page_grant_is_handed_out_again_while_it_applies() {
    let c = change(State::Checked, Some(HEAD));
    let (mut g, mut pages) = (Grants::default(), grants::Pages::default());
    let first = page(&mut g, &mut pages, &c, "change.approve", "one", 100);
    assert!(!first.reused);
    assert_eq!(first.grant.expires, 100 + grants::PAGE_TTL);
    let again = page(&mut g, &mut pages, &c, "change.approve", "two", 400);
    assert!(again.reused);
    assert_eq!(again.token, "one", "the same link, not a new one");
    assert_eq!(g.0.len(), 1, "nothing new is minted");
    assert!(g.peek("one", 7, 400).is_ok(), "the first link still works");
}

#[test]
fn a_moved_head_mints_a_new_page_grant_and_voids_the_old() {
    let c = change(State::Checked, Some(HEAD));
    let (mut g, mut pages) = (Grants::default(), grants::Pages::default());
    page(&mut g, &mut pages, &c, "change.approve", "old", 0);
    let moved = change(State::Checked, Some(NEXT));
    let fresh = page(&mut g, &mut pages, &moved, "change.approve", "new", 10);
    assert!(!fresh.reused);
    assert_eq!(fresh.token, "new");
    assert_eq!(fresh.grant.head.as_deref(), Some(NEXT));
    let why = kept!(g.redeem("old", "change.approve", &moved, 20)).expect_err("void");
    assert!(why.contains("moved since this link was made"), "{why}");
    assert!(kept!(g.redeem("new", "change.approve", &moved, 20)).is_ok());
}

#[test]
fn a_used_page_grant_is_replaced_by_a_new_one() {
    let c = change(State::Checked, Some(HEAD));
    let (mut g, mut pages) = (Grants::default(), grants::Pages::default());
    page(&mut g, &mut pages, &c, "change.approve", "one", 0);
    kept!(g.redeem("one", "change.approve", &c, 5)).expect("redeems");
    let next = page(&mut g, &mut pages, &c, "change.approve", "two", 10);
    assert!(!next.reused);
    assert_eq!(next.token, "two");
    assert!(g.peek("one", 7, 10).is_err(), "a link is used once");
}

#[test]
fn a_page_grant_near_its_end_or_of_another_tier_is_minted_anew() {
    use agent::core::approval::listed::REISSUE_BEFORE;
    use agent::core::approval::tiers::{Tier, Tiered, Tiers};
    let mut c = change(State::Checked, Some(HEAD));
    let (mut g, mut pages) = (Grants::default(), grants::Pages::default());
    page(&mut g, &mut pages, &c, "change.approve", "one", 0);
    let late = grants::PAGE_TTL - REISSUE_BEFORE;
    let renewed = page(&mut g, &mut pages, &c, "change.approve", "two", late);
    assert!(!renewed.reused, "under {REISSUE_BEFORE}s left, a new link");
    assert!(
        g.peek("one", 7, late).is_ok(),
        "the old link works until it ends"
    );
    let tiered = |tier| Tiered {
        tier,
        rule: "r".into(),
    };
    c.tiers = Some(Tiers {
        authorize: tiered(Tier::Code),
        merge: tiered(Tier::Owner),
        close: tiered(Tier::Confirm),
    });
    let raised = page(&mut g, &mut pages, &c, "change.approve", "three", late + 1);
    assert!(!raised.reused, "a new tier, a new link");
    assert_eq!(raised.grant.tier, Some(Tier::Owner));
}

#[test]
fn each_operation_has_its_own_page_grant() {
    let c = change(State::Checked, Some(HEAD));
    let (mut g, mut pages) = (Grants::default(), grants::Pages::default());
    page(&mut g, &mut pages, &c, "change.approve", "a", 0);
    let close = page(&mut g, &mut pages, &c, "change.close", "b", 1);
    assert!(!close.reused);
    assert_eq!(
        page(&mut g, &mut pages, &c, "change.approve", "c", 2).token,
        "a"
    );
    assert_eq!(
        page(&mut g, &mut pages, &c, "change.close", "d", 3).token,
        "b"
    );
}
