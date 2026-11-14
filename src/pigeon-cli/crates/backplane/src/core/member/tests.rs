use super::*;

const ROLES: &str = r#"{"roles": {"administrator": ["read", "propose", "approve", "approve.owner", "auto", "mandates", "platform", "members"], "user": [], "service": ["read", "propose"]}}"#;

fn roles() -> Roles {
    serde_json::from_str(ROLES).unwrap()
}

fn roles_with(f: impl FnOnce(&mut Roles)) -> Roles {
    let mut r = roles();
    f(&mut r);
    r
}

fn seeded() -> Members {
    Members::default().seed("root", 100)
}

/// `root` plus an active user `bob`, admitted through an invitation.
fn with_bob() -> Members {
    let (m, _) = seeded()
        .invite("t1", "bob@example.com", Role::User, "root", 100)
        .unwrap();
    let (m, inv) = m.enrol("t1", "bob", "pk", b"key", 101).unwrap();
    m.admit(&inv.id, "root", 102).unwrap().0
}

fn req<'a>(account: &'a str, capability: &'a str) -> Request<'a> {
    Request {
        account,
        capability,
        needs_proof: false,
        proof: None,
        proposer: None,
        subject: None,
    }
}

#[test]
fn roles_json_parses_and_passes() {
    let r: Roles = serde_json::from_str(ROLES).unwrap();
    assert_eq!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_a_fourth_role() {
    let r = roles_with(|r| {
        r.roles.insert("guest".into(), vec![]);
    });
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_a_missing_role() {
    let r = roles_with(|r| {
        r.roles.remove("user");
    });
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_an_unknown_capability() {
    let r = roles_with(|r| r.roles.get_mut("user").unwrap().push("fly".into()));
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_a_wildcard() {
    let r = roles_with(|r| r.roles.get_mut("user").unwrap().push("*".into()));
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_an_administrator_missing_a_capability() {
    let r = roles_with(|r| {
        r.roles
            .get_mut("administrator")
            .unwrap()
            .retain(|c| c != "members");
    });
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_a_service_above_the_ceiling() {
    let r = roles_with(|r| r.roles.get_mut("service").unwrap().push("approve".into()));
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn roles_refuse_an_unknown_field() {
    assert!(serde_json::from_str::<Roles>(r#"{"roles": {}, "extra": 1}"#).is_err());
}

#[test]
fn seed_makes_one_administrator_once() {
    let m = seeded();
    assert_eq!(m.memberships.len(), 1);
    assert_eq!(m.memberships[0].role, Role::Administrator);
    assert_eq!(m.memberships[0].admitted_by, SEED);
    let m = m.seed("other", 200);
    assert_eq!(m.memberships.len(), 1);
    assert_eq!(m.memberships[0].account, "root");
}

#[test]
fn invite_needs_an_active_administrator() {
    let err = with_bob()
        .invite("t2", "c@example.com", Role::User, "bob", 200)
        .unwrap_err();
    assert_eq!(err, "only an administrator may do this");
    assert!(
        seeded()
            .invite("t2", "c@example.com", Role::User, "nobody", 200)
            .is_err()
    );
}

#[test]
fn invite_refuses_the_service_role() {
    let err = seeded()
        .invite("t", "a@example.com", Role::Service, "root", 100)
        .unwrap_err();
    assert_eq!(err, "a service account is created, not invited");
}

#[test]
fn invite_refuses_a_bad_email() {
    for bad in ["", "a", "@b", "a@", "a@b@c"] {
        let err = seeded()
            .invite("t", bad, Role::User, "root", 100)
            .unwrap_err();
        assert_eq!(err, "not an email address", "{bad}");
    }
    let long = format!("{}@example.com", "a".repeat(250));
    assert!(
        seeded()
            .invite("t", &long, Role::User, "root", 100)
            .is_err()
    );
}

#[test]
fn invite_refuses_an_eleventh_open_invitation() {
    let mut m = seeded();
    for i in 0..MAX_OPEN_INVITES {
        m = m
            .invite(&format!("t{i}"), "a@example.com", Role::User, "root", 100)
            .unwrap()
            .0;
    }
    let err = m
        .invite("t-last", "a@example.com", Role::User, "root", 100)
        .unwrap_err();
    assert_eq!(err, "at most 10 invitations are open");
}

#[test]
fn enrol_works_once() {
    let (m, _) = seeded()
        .invite("t", "A@Example.com ", Role::User, "root", 100)
        .unwrap();
    let (m, inv) = m.enrol("t", "alice", "pk", b"key", 101).unwrap();
    assert_eq!(inv.email, "a@example.com");
    assert_eq!(inv.status, InviteStatus::Enrolled);
    assert_eq!(inv.account.as_deref(), Some("alice"));
    assert_eq!(inv.passkey.as_deref(), Some("pk"));
    assert!(inv.match_code.is_some());
    assert_eq!(
        m.clone()
            .enrol("t", "alice", "pk", b"key", 102)
            .unwrap_err(),
        "this link was already used"
    );
    assert_eq!(
        m.enrol("nope", "alice", "pk", b"key", 102).unwrap_err(),
        "no such invitation"
    );
}

#[test]
fn enrol_refuses_an_expired_link() {
    let (m, _) = seeded()
        .invite("t", "a@example.com", Role::User, "root", 100)
        .unwrap();
    let err = m
        .enrol("t", "alice", "pk", b"key", 100 + INVITE_TTL)
        .unwrap_err();
    assert_eq!(err, "this link has expired");
}

#[test]
fn enrol_refuses_an_active_member() {
    let (m, _) = with_bob()
        .invite("t2", "b@example.com", Role::User, "root", 200)
        .unwrap();
    let err = m.enrol("t2", "bob", "pk", b"key", 201).unwrap_err();
    assert_eq!(err, "already a member");
}

#[test]
fn match_code_is_six_digits_and_depends_on_token_and_key() {
    let c = match_code("t", b"key");
    assert_eq!(c.len(), 6);
    assert!(c.bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(c, match_code("t", b"key"));
    assert_ne!(c, match_code("u", b"key"));
    assert_ne!(c, match_code("t", b"other"));
}

#[test]
fn admit_needs_an_enrolled_invitation() {
    let (m, _) = seeded()
        .invite("t", "a@example.com", Role::User, "root", 100)
        .unwrap();
    let id = token_id("t");
    assert_eq!(
        m.clone().admit(&id, "root", 101).unwrap_err(),
        "not enrolled"
    );
    assert_eq!(
        m.admit("missing", "root", 101).unwrap_err(),
        "no such invitation"
    );
}

#[test]
fn admit_refuses_the_invitee_admitting_themselves() {
    let (mut m, _) = seeded()
        .invite("t", "a@example.com", Role::User, "root", 100)
        .unwrap();
    m.invitations[0].status = InviteStatus::Enrolled;
    m.invitations[0].account = Some("root".into());
    let err = m.admit(&token_id("t"), "root", 101).unwrap_err();
    assert_eq!(err, "no one admits themselves");
}

#[test]
fn admit_makes_an_active_membership() {
    let m = with_bob();
    let bob = m.active("bob").unwrap();
    assert_eq!(bob.role, Role::User);
    assert_eq!(bob.admitted_by, "root");
    assert_eq!(bob.at, 102);
    assert_eq!(bob.scopes, Vec::<String>::new());
    assert_eq!(m.invitations[0].status, InviteStatus::Admitted);
}

#[test]
fn set_role_refuses_ones_own_role() {
    let err = seeded()
        .set_role("root", Role::User, "root", 200)
        .unwrap_err();
    assert_eq!(err, "no one changes their own role");
    let (m, bob) = with_bob()
        .set_role("bob", Role::Administrator, "root", 200)
        .unwrap();
    assert_eq!(bob.role, Role::Administrator);
    assert_eq!(m.active("bob").unwrap().at, 200);
}

#[test]
fn remove_ends_a_membership_and_revokes_its_invitations() {
    let (m, _) = with_bob()
        .set_role("bob", Role::Administrator, "root", 150)
        .unwrap();
    let (m, _) = m
        .invite("t2", "c@example.com", Role::User, "bob", 200)
        .unwrap();
    let m = m.remove("bob", "root", 300).unwrap();
    assert!(m.active("bob").is_none());
    let bob = m.memberships.iter().find(|x| x.account == "bob").unwrap();
    assert_eq!(bob.status, Status::Ended);
    assert_eq!(bob.ended, Some(300));
    let inv = m
        .invitations
        .iter()
        .find(|i| i.id == token_id("t2"))
        .unwrap();
    assert_eq!(inv.status, InviteStatus::Revoked);
}

#[test]
fn remove_refuses_oneself() {
    let err = seeded().remove("root", "root", 200).unwrap_err();
    assert_eq!(err, "use leave to end your own membership");
}

#[test]
fn leave_refuses_the_last_administrator() {
    let err = seeded().leave("root", 200).unwrap_err();
    assert_eq!(err, "the last administrator can't leave");
}

#[test]
fn leave_lets_one_of_two_administrators_go() {
    let (m, _) = with_bob()
        .set_role("bob", Role::Administrator, "root", 150)
        .unwrap();
    let m = m.leave("root", 200).unwrap();
    assert!(m.active("root").is_none());
    assert_eq!(m.administrators(), 1);
}

#[test]
fn create_service_refuses_scopes_above_the_ceiling() {
    let err = seeded()
        .create_service("svc", &["approve".to_owned()], "root", 200)
        .unwrap_err();
    assert_eq!(err, "a service account's scopes are read and propose only");
    assert!(seeded().create_service("svc", &[], "root", 200).is_err());
    let (m, svc) = seeded()
        .create_service("svc", &["read".to_owned()], "root", 200)
        .unwrap();
    assert_eq!(svc.role, Role::Service);
    assert!(m.active("svc").is_some());
}

#[test]
fn check_refuses_a_non_member() {
    let m = seeded();
    assert_eq!(
        check(&roles(), &m, &req("nobody", "read")),
        Err(Refusal::NotMember)
    );
}

#[test]
fn check_refuses_a_user_every_capability() {
    let m = with_bob();
    for c in CAPABILITIES {
        assert_eq!(
            check(&roles(), &m, &req("bob", c)),
            Err(Refusal::Capability),
            "{c}"
        );
    }
}

#[test]
fn check_allows_an_administrator_every_capability() {
    let m = seeded();
    for c in CAPABILITIES {
        assert!(check(&roles(), &m, &req("root", c)).is_ok(), "{c}");
    }
}

#[test]
fn check_holds_a_service_to_its_scopes_and_the_ceiling() {
    let (m, _) = seeded()
        .create_service("svc", &["read".to_owned()], "root", 200)
        .unwrap();
    assert!(check(&roles(), &m, &req("svc", "read")).is_ok());
    assert_eq!(
        check(&roles(), &m, &req("svc", "propose")),
        Err(Refusal::Capability)
    );
    assert_eq!(
        check(&roles(), &m, &req("svc", "approve")),
        Err(Refusal::Capability)
    );
    let wide = roles_with(|r| r.roles.get_mut("service").unwrap().push("approve".into()));
    let mut m = m;
    m.memberships[1].scopes = vec!["approve".into()];
    assert_eq!(
        check(&wide, &m, &req("svc", "approve")),
        Err(Refusal::Capability)
    );
}

#[test]
fn check_refuses_a_proof_by_another_account() {
    let m = seeded();
    let mut r = req("root", "approve");
    r.needs_proof = true;
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Proof));
    r.proof = Some("bob");
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Proof));
    r.proof = Some("root");
    assert!(check(&roles(), &m, &r).is_ok());
}

#[test]
fn check_refuses_a_service_any_proof() {
    let (m, _) = seeded()
        .create_service("svc", &["read".to_owned()], "root", 200)
        .unwrap();
    let mut r = req("svc", "read");
    r.needs_proof = true;
    r.proof = Some("svc");
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Proof));
}

#[test]
fn check_refuses_the_proposer() {
    let m = seeded();
    let mut r = req("root", "approve");
    r.proposer = Some("root");
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Separation));
    r.proposer = Some("bob");
    assert!(check(&roles(), &m, &r).is_ok());
}

#[test]
fn check_refuses_ones_own_membership() {
    let m = seeded();
    let mut r = req("root", "members");
    r.subject = Some("root");
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Separation));
    r.subject = Some("bob");
    assert!(check(&roles(), &m, &r).is_ok());
}
