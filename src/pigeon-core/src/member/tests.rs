use super::*;

const ROLES: &str = r#"{"roles": {"administrator": ["read", "propose", "approve", "approve.owner", "auto", "mandates", "platform", "members"], "standard": [], "service": ["read", "propose"]}}"#;

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
        .invite("t1", "bob@example.com", Role::Standard, "root", 100)
        .unwrap();
    let (m, inv) = m.enrol("t1", "bob", "pk", b"key", 101).unwrap();
    m.admit(&inv.id, "root", 102).unwrap().0
}

/// `with_bob` with `bob` an administrator too, as stored before the one-Administrator rule.
fn two_admins() -> Members {
    let mut m = with_bob();
    m.memberships[1].role = Role::Administrator;
    m
}

fn req<'a>(account: &'a str, p: &'static str) -> Request<'a> {
    Request {
        account,
        requires: Requires::Permission(p),
        scope: None,
        now: 1000,
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
        r.roles.remove("standard");
    });
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_an_unknown_capability() {
    let r = roles_with(|r| r.roles.get_mut("standard").unwrap().push("fly".into()));
    assert_ne!(check_roles(&r), Vec::<String>::new());
}

#[test]
fn check_roles_refuses_a_wildcard() {
    let r = roles_with(|r| r.roles.get_mut("standard").unwrap().push("*".into()));
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
        .invite("t2", "c@example.com", Role::Standard, "bob", 200)
        .unwrap_err();
    assert_eq!(err, "only an administrator may do this");
    assert!(
        seeded()
            .invite("t2", "c@example.com", Role::Standard, "nobody", 200)
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
            .invite("t", bad, Role::Standard, "root", 100)
            .unwrap_err();
        assert_eq!(err, "not an email address", "{bad}");
    }
    let long = format!("{}@example.com", "a".repeat(250));
    assert!(
        seeded()
            .invite("t", &long, Role::Standard, "root", 100)
            .is_err()
    );
}

#[test]
fn invite_refuses_an_eleventh_open_invitation() {
    let mut m = seeded();
    for i in 0..MAX_OPEN_INVITES {
        m = m
            .invite(
                &format!("t{i}"),
                "a@example.com",
                Role::Standard,
                "root",
                100,
            )
            .unwrap()
            .0;
    }
    let err = m
        .invite("t-last", "a@example.com", Role::Standard, "root", 100)
        .unwrap_err();
    assert_eq!(err, "at most 10 invitations are open");
}

#[test]
fn enrol_works_once() {
    let (m, _) = seeded()
        .invite("t", "A@Example.com ", Role::Standard, "root", 100)
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
        .invite("t", "a@example.com", Role::Standard, "root", 100)
        .unwrap();
    let err = m
        .enrol("t", "alice", "pk", b"key", 100 + INVITE_TTL)
        .unwrap_err();
    assert_eq!(err, "this link has expired");
}

#[test]
fn enrol_refuses_an_active_member() {
    let (m, _) = with_bob()
        .invite("t2", "b@example.com", Role::Standard, "root", 200)
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
        .invite("t", "a@example.com", Role::Standard, "root", 100)
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
        .invite("t", "a@example.com", Role::Standard, "root", 100)
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
    assert_eq!(bob.role, Role::Standard);
    assert_eq!(bob.admitted_by, "root");
    assert_eq!(bob.at, 102);
    assert_eq!(bob.scopes, Vec::<String>::new());
    assert_eq!(m.invitations[0].status, InviteStatus::Admitted);
}

#[test]
fn set_role_refuses_ones_own_role() {
    let err = seeded()
        .set_role("root", Role::Standard, "root", 200)
        .unwrap_err();
    assert_eq!(err, "no one changes their own role");
    let err = with_bob()
        .set_role("bob", Role::Administrator, "root", 200)
        .unwrap_err();
    assert_eq!(err, ONE_ADMINISTRATOR);
}

#[test]
fn remove_ends_a_membership_and_revokes_its_invitations() {
    let (m, _) = two_admins()
        .invite("t2", "c@example.com", Role::Standard, "bob", 200)
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
    let m = two_admins().leave("root", 200).unwrap();
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
fn check_refuses_a_standard_member_every_permission() {
    let m = with_bob();
    for c in PERMISSIONS {
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
    for c in PERMISSIONS {
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

#[test]
fn view_shows_a_non_member_nothing() {
    let v = with_bob().view("stranger");
    assert_eq!(v.me, None);
    assert_eq!(v.members, Vec::<Membership>::new());
    assert_eq!(v.invitations, Vec::<Invitation>::new());
}

#[test]
fn view_shows_a_user_only_their_own_row() {
    let (m, _) = with_bob()
        .invite("t2", "c@example.com", Role::Standard, "root", 200)
        .unwrap();
    let v = m.view("bob");
    assert_eq!(v.me.as_ref().map(|m| m.account.as_str()), Some("bob"));
    assert_eq!(v.members, Vec::<Membership>::new());
    assert_eq!(v.invitations, Vec::<Invitation>::new());
}

#[test]
fn view_shows_an_administrator_every_active_member_and_open_invitation() {
    let (m, _) = with_bob()
        .invite("t2", "c@example.com", Role::Standard, "root", 200)
        .unwrap();
    let (m, _) = m
        .invite("t3", "d@example.com", Role::Standard, "root", 200)
        .unwrap();
    let (m, _) = m.enrol("t3", "dee", "pk2", b"key2", 201).unwrap();
    let v = m.view("root");
    assert_eq!(v.me.as_ref().map(|m| m.account.as_str()), Some("root"));
    let accounts: Vec<&str> = v.members.iter().map(|m| m.account.as_str()).collect();
    assert_eq!(accounts, ["root", "bob"]);
    let statuses: Vec<InviteStatus> = v.invitations.iter().map(|i| i.status).collect();
    assert_eq!(statuses, [InviteStatus::Open, InviteStatus::Enrolled]);
}

#[test]
fn view_hides_ended_memberships_and_closed_invitations() {
    let (m, inv) = with_bob()
        .invite("t2", "c@example.com", Role::Standard, "root", 200)
        .unwrap();
    let m = m.revoke(&inv.id, "root").unwrap();
    let m = m.remove("bob", "root", 300).unwrap();
    let v = m.view("root");
    let accounts: Vec<&str> = v.members.iter().map(|m| m.account.as_str()).collect();
    assert_eq!(accounts, ["root"]);
    assert_eq!(v.invitations, Vec::<Invitation>::new());
    assert_eq!(m.view("bob").me, None);
}

#[test]
fn the_repository_roles_file_parses_and_passes() {
    let text = include_str!("../../../../.config/roles.yaml");
    let r = Roles::parse_file(text).unwrap();
    assert_eq!(r, roles());
    assert_eq!(check_roles(&r), Vec::<String>::new());
    assert_eq!(r.permissions(Role::Standard), Vec::<String>::new());
}

#[test]
fn standard_is_read_from_user() {
    assert_eq!(
        serde_json::from_str::<Role>(r#""user""#).unwrap(),
        Role::Standard
    );
    assert_eq!(Role::parse("user"), Ok(Role::Standard));
    assert_eq!(Role::parse("standard"), Ok(Role::Standard));
    assert_eq!(
        serde_json::to_string(&Role::Standard).unwrap(),
        r#""standard""#
    );
}

#[test]
fn check_roles_refuses_a_standard_above_its_ceiling() {
    let r = roles_with(|r| r.roles.get_mut("standard").unwrap().push("read".into()));
    assert!(
        check_roles(&r)
            .iter()
            .any(|p| p == "`standard` may not hold `read`")
    );
}

#[test]
fn no_one_is_invited_admitted_or_promoted_to_administrator() {
    let err = seeded()
        .invite("t", "a@example.com", Role::Administrator, "root", 100)
        .unwrap_err();
    assert_eq!(err, ONE_ADMINISTRATOR);
    let err = with_bob()
        .set_role("bob", Role::Administrator, "root", 200)
        .unwrap_err();
    assert_eq!(err, ONE_ADMINISTRATOR);
    let (mut m, _) = seeded()
        .invite("t", "a@example.com", Role::Standard, "root", 100)
        .unwrap();
    m.invitations[0].role = Role::Administrator;
    let (m, _) = m.enrol("t", "alice", "pk", b"key", 101).unwrap();
    let err = m.admit(&token_id("t"), "root", 102).unwrap_err();
    assert_eq!(err, ONE_ADMINISTRATOR);
}

#[test]
fn grant_needs_the_administrator_and_a_standard_member() {
    let err = with_bob()
        .grant("bob", "icbc", None, None, "bob", 200)
        .unwrap_err();
    assert_eq!(err, "only an administrator may do this");
    let err = seeded()
        .grant("root", "icbc", None, None, "root", 200)
        .unwrap_err();
    assert_eq!(err, "only a standard member is granted a capability");
    let err = with_bob()
        .grant("nobody", "icbc", None, None, "root", 200)
        .unwrap_err();
    assert_eq!(err, "only a standard member is granted a capability");
    let (m, g) = with_bob()
        .grant("bob", "icbc", None, None, "root", 200)
        .unwrap();
    assert_eq!(g.granted_by, "root");
    assert_eq!(m.active("bob").unwrap().grants, [g]);
}

#[test]
fn grant_refuses_an_ungrantable_capability_a_scope_on_icbc_and_a_past_expiry() {
    let err = with_bob()
        .grant("bob", "fly", None, None, "root", 200)
        .unwrap_err();
    assert_eq!(err, "`fly` is not a capability");
    let err = with_bob()
        .grant("bob", "blog", None, None, "root", 200)
        .unwrap_err();
    assert_eq!(err, "`blog` can't be granted yet");
    let err = with_bob()
        .grant("bob", "icbc", Some("x"), None, "root", 200)
        .unwrap_err();
    assert_eq!(err, "`icbc` takes no scope");
    for t in [199, 200] {
        let err = with_bob()
            .grant("bob", "icbc", None, Some(t), "root", 200)
            .unwrap_err();
        assert_eq!(err, "a grant must expire in the future");
    }
}

#[test]
fn a_grant_holds_until_it_expires_or_is_revoked() {
    let (m, _) = with_bob()
        .grant("bob", "icbc", None, Some(300), "root", 200)
        .unwrap();
    assert!(m.holds("bob", "icbc", None, 299));
    assert!(!m.holds("bob", "icbc", None, 300));
    assert!(!m.holds("bob", "blog", None, 250));
    let (m, _) = with_bob()
        .grant("bob", "icbc", None, None, "root", 200)
        .unwrap();
    assert!(m.holds("bob", "icbc", None, u64::MAX));
    assert_eq!(
        m.clone()
            .revoke_grant("bob", "icbc", "bob", 250)
            .unwrap_err(),
        "only an administrator may do this"
    );
    assert_eq!(
        m.clone()
            .revoke_grant("nobody", "icbc", "root", 250)
            .unwrap_err(),
        "not a member"
    );
    let m = m.revoke_grant("bob", "icbc", "root", 250).unwrap();
    assert!(!m.holds("bob", "icbc", None, 251));
    assert_eq!(
        m.revoke_grant("bob", "icbc", "root", 260).unwrap_err(),
        "no such grant"
    );
}

#[test]
fn regranting_replaces_the_active_grant() {
    let (m, _) = with_bob()
        .grant("bob", "icbc", None, Some(300), "root", 200)
        .unwrap();
    let (m, g) = m.grant("bob", "icbc", None, None, "root", 250).unwrap();
    let grants = &m.active("bob").unwrap().grants;
    assert!(matches!(grants.as_slice(), [_, _]));
    assert_eq!(grants[0].revoked, Some(250));
    assert_eq!(grants[1], g);
    assert!(m.holds("bob", "icbc", None, 400));
}

#[test]
fn ending_a_membership_revokes_its_grants() {
    let (m, _) = with_bob()
        .grant("bob", "icbc", None, None, "root", 200)
        .unwrap();
    let m = m.remove("bob", "root", 300).unwrap();
    let bob = m.memberships.iter().find(|x| x.account == "bob").unwrap();
    assert_eq!(bob.grants[0].revoked, Some(300));
    assert!(!m.holds("bob", "icbc", None, 301));
}

#[test]
fn check_passes_a_capability_only_through_an_active_grant_or_the_administrator() {
    let cap = |account| Request {
        requires: Requires::Capability {
            name: "icbc",
            scope_arg: None,
        },
        ..req(account, "read")
    };
    let m = with_bob();
    assert_eq!(check(&roles(), &m, &cap("bob")), Err(Refusal::Capability));
    assert!(check(&roles(), &m, &cap("root")).is_ok());
    let (m, _) = m.grant("bob", "icbc", None, None, "root", 200).unwrap();
    assert!(check(&roles(), &m, &cap("bob")).is_ok());
    let (m, _) = m
        .create_service("svc", &["read".to_owned()], "root", 200)
        .unwrap();
    assert_eq!(check(&roles(), &m, &cap("svc")), Err(Refusal::Capability));
}

#[test]
fn a_standard_member_with_every_grant_holds_no_permission() {
    let (m, _) = with_bob()
        .grant("bob", "icbc", None, None, "root", 200)
        .unwrap();
    for c in PERMISSIONS {
        assert_eq!(
            check(&roles(), &m, &req("bob", c)),
            Err(Refusal::Capability),
            "{c}"
        );
    }
}

#[test]
fn check_holds_own_to_the_callers_subject() {
    let mut r = Request {
        requires: Requires::Own,
        ..req("bob", "read")
    };
    let m = with_bob();
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Capability));
    r.subject = Some("root");
    assert_eq!(check(&roles(), &m, &r), Err(Refusal::Capability));
    r.subject = Some("bob");
    assert!(check(&roles(), &m, &r).is_ok());
}

#[test]
fn migrate_grants_icbc_only_on_a_learner_instance_and_never_over_a_revocation() {
    let (m, added) = with_bob().migrate(false, 500);
    assert!(!added);
    assert_eq!(m.active("bob").unwrap().grants, Vec::<Grant>::new());
    let (m, added) = m.migrate(true, 500);
    assert!(added);
    let g = &m.active("bob").unwrap().grants;
    assert!(matches!(g.as_slice(), [_]));
    assert_eq!(g[0].granted_by, MIGRATION);
    assert_eq!(m.active("root").unwrap().grants, Vec::<Grant>::new());
    assert!(m.holds("bob", "icbc", None, 501));
    let (m, added) = m.migrate(true, 600);
    assert!(!added);
    assert!(matches!(m.active("bob").unwrap().grants.as_slice(), [_]));
    let m = m.revoke_grant("bob", "icbc", "root", 700).unwrap();
    let (m, added) = m.migrate(true, 800);
    assert!(!added);
    assert!(!m.holds("bob", "icbc", None, 801));
}

#[test]
fn parse_file_refuses_another_shape() {
    assert!(Roles::parse_file("roles:\n  standard: learner\n").is_err());
    assert!(Roles::parse_file("roles:\nstandard: [learner]\n").is_err());
}
