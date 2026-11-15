use super::*;

const NOW: u64 = 1_000_000;

fn change(state: State, head: Option<&str>) -> Change {
    let mut c = Change::propose(7, "x", "feat: x", "p", vec!["**".into()], "agent");
    c.state = state;
    c.head = head.map(str::to_string);
    c
}

fn stage(needs: Needs) -> Stage {
    Stage {
        stage: "checked".into(),
        needs,
        waits_on: None,
        next: "approve the head".into(),
        since: None,
    }
}

fn owed_of(operation: &'static str, head: &str) -> Owed {
    Owed {
        number: 7,
        operation,
        head: Some(head.into()),
    }
}

fn listed_of(operation: &str, head: &str, expires: u64) -> Listed {
    Listed {
        operation: operation.into(),
        head: Some(head.into()),
        expires,
    }
}

#[test]
fn a_checked_change_needing_the_owner_owes_an_approve_of_its_head() {
    let c = change(State::Checked, Some("5f1bccf"));
    assert_eq!(
        owed(&c, &stage(Needs::Owner)),
        Some(owed_of("change.approve", "5f1bccf"))
    );
}

#[test]
fn a_proposed_change_needing_the_owner_owes_an_authorize_before_any_push() {
    let c = change(State::Proposed, None);
    let o = owed(&c, &stage(Needs::Owner)).expect("owed");
    assert_eq!((o.operation, o.head.as_deref()), ("change.authorize", None));
    assert_eq!(reconcile(Some(&o), None, NOW), Action::Issue { push: true });
}

#[test]
fn a_checked_change_without_a_head_owes_nothing() {
    let c = change(State::Checked, None);
    assert_eq!(owed(&c, &stage(Needs::Owner)), None);
}

#[test]
fn a_covered_change_owes_nothing() {
    let c = change(State::Checked, Some("5f1bccf"));
    assert_eq!(owed(&c, &stage(Needs::Agent)), None);
}

#[test]
fn an_approved_change_owes_nothing() {
    let c = change(State::Approved, Some("5f1bccf"));
    assert_eq!(owed(&c, &stage(Needs::Owner)), None);
}

#[test]
fn a_failed_deploy_owes_no_grant() {
    let c = change(State::Merged, Some("5f1bccf"));
    assert_eq!(owed(&c, &stage(Needs::Owner)), None);
}

#[test]
fn nothing_owed_and_nothing_listed_is_kept() {
    assert_eq!(reconcile(None, None, NOW), Action::Keep);
}

#[test]
fn nothing_owed_but_listed_is_removed() {
    let l = listed_of("change.approve", "5f1bccf", NOW + 20 * 3600);
    assert_eq!(reconcile(None, Some(&l), NOW), Action::Remove);
}

#[test]
fn case_464_checked_with_nothing_listed_is_issued_and_pushed() {
    let o = owed_of("change.approve", "5f1bccf");
    assert_eq!(reconcile(Some(&o), None, NOW), Action::Issue { push: true });
}

#[test]
fn case_464_a_session_push_to_a_new_head_is_issued_and_pushed() {
    let o = owed_of("change.approve", "efd44b8");
    let l = listed_of("change.approve", "5f1bccf", NOW + 20 * 3600);
    assert_eq!(
        reconcile(Some(&o), Some(&l), NOW),
        Action::Issue { push: true }
    );
}

#[test]
fn case_438_an_entry_for_a_head_no_longer_current_is_reissued() {
    let o = owed_of("change.approve", "bedac41");
    let l = listed_of("change.approve", "0a1b2c3", NOW + 20 * 3600);
    assert_eq!(
        reconcile(Some(&o), Some(&l), NOW),
        Action::Issue { push: true }
    );
}

#[test]
fn a_listed_entry_for_another_operation_is_reissued() {
    let o = owed_of("change.approve", "5f1bccf");
    let l = listed_of("change.authorize", "5f1bccf", NOW + 20 * 3600);
    assert_eq!(
        reconcile(Some(&o), Some(&l), NOW),
        Action::Issue { push: true }
    );
}

#[test]
fn a_current_entry_with_time_left_is_kept() {
    let o = owed_of("change.approve", "5f1bccf");
    let l = listed_of("change.approve", "5f1bccf", NOW + REISSUE_BEFORE + 1);
    assert_eq!(reconcile(Some(&o), Some(&l), NOW), Action::Keep);
}

#[test]
fn an_entry_within_reissue_before_is_reissued_without_a_push() {
    let o = owed_of("change.approve", "5f1bccf");
    let l = listed_of("change.approve", "5f1bccf", NOW + REISSUE_BEFORE);
    assert_eq!(
        reconcile(Some(&o), Some(&l), NOW),
        Action::Issue { push: false }
    );
}

#[test]
fn an_expired_entry_is_reissued_without_a_push() {
    let o = owed_of("change.approve", "5f1bccf");
    let l = listed_of("change.approve", "5f1bccf", NOW - 1);
    assert_eq!(
        reconcile(Some(&o), Some(&l), NOW),
        Action::Issue { push: false }
    );
}
