//! Device links and signed requests: what each accepts, and what a refusal keeps.

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};

use super::devices::{
    Devices, IDLE, LINK_TTL, SKEW, canonical, clean_name, fingerprint, key_point, link_message,
    parse_header,
};
use crate::push::b64url;

const NOW: u64 = 1_800_000_000;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("a key")
}

fn public(k: &SigningKey) -> String {
    b64url(k.verifying_key().to_encoded_point(false).as_bytes())
}

fn sign(k: &SigningKey, data: &str) -> String {
    let s: Signature = k.sign(data.as_bytes());
    b64url(&s.to_bytes())
}

fn linked(k: &SigningKey) -> (Devices, String) {
    let d = Devices::default()
        .offer("Willow's iPhone", &public(k), "code-1", NOW)
        .expect("offered");
    let (d, r) = d.redeem("code-1", &public(k), &sign(k, &link_message("code-1")), NOW);
    (d, r.expect("linked").id)
}

fn header(k: &SigningKey, id: &str, method: &str, path: &str, at: u64, body: &[u8]) -> String {
    format!(
        "id={id};t={at};s={}",
        sign(k, &canonical(method, path, at, body))
    )
}

#[test]
fn a_proved_link_is_redeemed_once_by_the_key_it_was_proved_for() {
    let k = key(7);
    let (d, id) = linked(&k);
    let point = key_point(&public(&k)).expect("point");
    assert_eq!(id, fingerprint(&point));
    assert_eq!(d.devices.len(), 1);
    assert_eq!(d.devices[0].name, "Willow's iPhone");
    assert!(d.offers.is_empty(), "the offer is used up");
    let (_, again) = d.redeem(
        "code-1",
        &public(&k),
        &sign(&k, &link_message("code-1")),
        NOW,
    );
    assert!(again.is_err(), "a code links once");
}

#[test]
fn a_link_is_refused_for_another_key_a_bad_signature_or_after_it_lapses() {
    let (k, other) = (key(7), key(8));
    let offered = || {
        Devices::default()
            .offer("phone", &public(&k), "c", NOW)
            .expect("offered")
    };
    let cases: Vec<(&str, Devices, String, String, u64)> = vec![
        (
            "another key",
            offered(),
            public(&other),
            sign(&other, &link_message("c")),
            NOW,
        ),
        (
            "signed by another key",
            offered(),
            public(&k),
            sign(&other, &link_message("c")),
            NOW,
        ),
        (
            "signed over another code",
            offered(),
            public(&k),
            sign(&k, &link_message("d")),
            NOW,
        ),
        (
            "lapsed",
            offered(),
            public(&k),
            sign(&k, &link_message("c")),
            NOW + LINK_TTL,
        ),
    ];
    for (what, d, key, sig, at) in cases {
        let (after, r) = d.redeem("c", &key, &sig, at);
        assert!(r.is_err(), "{what}");
        assert!(after.devices.is_empty(), "{what}");
        assert!(after.offers.is_empty(), "{what}: the offer is used up");
    }
}

#[test]
fn a_signed_request_names_its_device_once() {
    let k = key(7);
    let (d, id) = linked(&k);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW + 5, b"");
    let (d, r) = d.check(&h, "GET", "/v0/app/home", b"", NOW + 5);
    assert_eq!(r.expect("checked").id, id);
    assert_eq!(d.devices[0].seen, NOW + 5);
    let (_, replay) = d.check(&h, "GET", "/v0/app/home", b"", NOW + 6);
    assert!(replay.expect_err("replayed").contains("replayed"));
}

#[test]
fn a_worker_retry_of_one_delivery_gets_the_same_answer() {
    let k = key(7);
    let (d, id) = linked(&k);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW, b"");
    // The first attempt is recorded, then its answer is lost as the object resets.
    let (d, first) = d.check_attempt(&h, "GET", "/v0/app/home", b"", "a1", NOW);
    assert_eq!(first.expect("checked").id, id);
    let (d, retry) = d.check_attempt(&h, "GET", "/v0/app/home", b"", "a1", NOW + 1);
    assert_eq!(retry.expect("the retry is the same delivery").id, id);
    assert_eq!(d.seen.len(), 1, "recorded once");
    // Another delivery of the same header is still a replay, as is one without an attempt.
    let (d, again) = d.check_attempt(&h, "GET", "/v0/app/home", b"", "a2", NOW + 2);
    assert!(again.expect_err("replayed").contains("replayed"));
    let (_, bare) = d.check(&h, "GET", "/v0/app/home", b"", NOW + 2);
    assert!(bare.expect_err("replayed").contains("replayed"));
}

#[test]
fn a_blank_attempt_never_matches_a_recorded_one() {
    let k = key(7);
    let (d, id) = linked(&k);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW, b"");
    let (d, r) = d.check(&h, "GET", "/v0/app/home", b"", NOW);
    assert!(r.is_ok());
    let (_, r) = d.check_attempt(&h, "GET", "/v0/app/home", b"", "", NOW);
    assert!(r.expect_err("replayed").contains("replayed"));
}

#[test]
fn requests_at_once_and_out_of_order_are_each_accepted_once() {
    let k = key(7);
    let (mut d, id) = linked(&k);
    let paths = [
        "/v0/app/home",
        "/v0/app/tracker",
        "/v0/app/ops",
        "/v0/app/approvals",
    ];
    let headers: Vec<_> = paths
        .iter()
        .enumerate()
        .map(|(n, p)| header(&k, &id, "GET", p, NOW + n as u64, b""))
        .collect();
    // Arriving in reverse, with the later timestamps first, each is its own request.
    for (n, (h, p)) in headers.iter().zip(paths).enumerate().rev() {
        let (next, r) = d.check_attempt(h, "GET", p, b"", &format!("a{n}"), NOW + 3);
        assert!(r.is_ok(), "{p}");
        d = next;
    }
    assert_eq!(d.seen.len(), paths.len());
    for (h, p) in headers.iter().zip(paths) {
        let (next, r) = d.check_attempt(h, "GET", p, b"", "later", NOW + 4);
        assert!(r.expect_err("replayed").contains("replayed"), "{p}");
        d = next;
    }
}

#[test]
fn a_seen_entry_from_before_attempts_still_reads() {
    let old = r#"{"devices":[],"offers":[],"seen":[{"sig":"ab","until":1}]}"#;
    let d: Devices = serde_json::from_str(old).expect("reads");
    assert_eq!(d.seen[0].attempt, "");
    let back = serde_json::to_string(&d).expect("writes");
    assert_eq!(back, old, "a blank attempt isn't written");
}

/// What a request case names: what differs, its header, method, path, body and time.
type Case<'a> = (&'a str, String, &'a str, &'a str, &'a [u8], u64);

#[test]
fn a_request_is_refused_when_anything_it_signed_differs() {
    let (k, other) = (key(7), key(8));
    let (d, id) = linked(&k);
    let body = br#"{"hold":false}"#;
    let ok = |at| header(&k, &id, "POST", "/v0/app/approvals/g", at, body);
    let cases: Vec<Case> = vec![
        (
            "another path",
            ok(NOW),
            "POST",
            "/v0/app/approvals/h",
            body,
            NOW,
        ),
        (
            "another method",
            ok(NOW),
            "PUT",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "another body",
            ok(NOW),
            "POST",
            "/v0/app/approvals/g",
            br#"{"hold":true}"#,
            NOW,
        ),
        (
            "too old",
            ok(NOW - SKEW - 1),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "from the future",
            ok(NOW + SKEW + 1),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "another key",
            header(&other, &id, "POST", "/v0/app/approvals/g", NOW, body),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "an unknown device",
            header(&k, "nobody", "POST", "/v0/app/approvals/g", NOW, body),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
        (
            "a malformed header",
            "id=x".into(),
            "POST",
            "/v0/app/approvals/g",
            body,
            NOW,
        ),
    ];
    for (what, h, method, path, b, at) in cases {
        let (after, r) = d.clone().check(&h, method, path, b, at);
        assert!(r.is_err(), "{what}");
        assert!(after.seen.is_empty(), "{what}: a refusal records nothing");
    }
}

#[test]
fn an_unlinked_or_idle_device_is_refused() {
    let k = key(7);
    let (d, id) = linked(&k);
    let gone = d.clone().unlink(&id);
    let h = header(&k, &id, "GET", "/v0/app/home", NOW, b"");
    assert!(gone.check(&h, "GET", "/v0/app/home", b"", NOW).1.is_err());
    let late = NOW + IDLE + 1;
    let h = header(&k, &id, "GET", "/v0/app/home", late, b"");
    assert!(d.check(&h, "GET", "/v0/app/home", b"", late).1.is_err());
}

#[test]
fn der_signatures_and_spki_keys_are_accepted_too() {
    let k = key(9);
    let spki = {
        let mut v = vec![
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ];
        v.extend_from_slice(k.verifying_key().to_encoded_point(false).as_bytes());
        b64url(&v)
    };
    let d = Devices::default()
        .offer("phone", &spki, "c", NOW)
        .expect("offered");
    let s: Signature = k.sign(link_message("c").as_bytes());
    let (_, r) = d.redeem("c", &spki, &b64url(s.to_der().as_bytes()), NOW);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn names_and_headers_are_read_strictly() {
    assert_eq!(clean_name("  "), "iPhone");
    assert_eq!(clean_name("a\u{0}b"), "ab");
    assert_eq!(clean_name(&"x".repeat(100)).len(), 64);
    assert_eq!(
        parse_header("id=a; t=12; s=sig").expect("parsed"),
        ("a".into(), 12, "sig".into())
    );
    assert!(parse_header("id=a;s=sig").is_err());
    assert!(key_point("not a key").is_err());
}

mod approvals {
    use serde_json::json;

    use crate::app::approvals::{detail, find, id, list, proof_form};
    use crate::approval::grants::{Grant, key};
    use crate::approve::Waiting;
    use crate::change::Change;

    fn waiting(number: u32, path: &str, expires: u64) -> Waiting {
        Waiting {
            number,
            operation: "change.approve".into(),
            title: format!("change {number}"),
            path: path.into(),
            expires,
        }
    }

    #[test]
    fn an_approval_is_named_by_its_grants_key_and_never_shows_its_token() {
        let w = vec![
            waiting(7, "/approve/7?grant=tok-seven", 100),
            waiting(9, "/approve/9?grant=tok-nine", 200),
            waiting(3, "/platform/3?grant=tok-platform", 300),
        ];
        let l = list(&w);
        let items = l.as_array().expect("a list");
        assert_eq!(items.len(), 2, "platform grants stay on the pages");
        assert_eq!(items[0]["number"], 9, "newest expiry first");
        assert_eq!(items[0]["id"], key("tok-nine"));
        assert!(!l.to_string().contains("tok-"), "no token leaves");
        let (found, token) = find(&w, &key("tok-seven")).expect("found");
        assert_eq!((found.number, token), (7, "tok-seven"));
        assert!(find(&w, &key("tok-platform")).is_none());
        assert!(find(&w, "nothing").is_none());
        assert_eq!(id(&w[2]), None);
    }

    #[test]
    fn the_detail_shows_the_card_and_the_proof_needed() {
        let w = waiting(7, "/approve/7?grant=tok", 100);
        let mut c = Change::propose(7, "x", "feat: x", "the plan", vec!["a/**".into()], "agent");
        c.head = Some("a".repeat(40));
        let g = Grant {
            operation: "change.approve".into(),
            number: 7,
            head: c.head.clone(),
            expires: 100,
            tier: None,
            used: None,
        };
        let d = detail(&w, &c, &g);
        assert_eq!(d["title"], "feat: x");
        assert_eq!(d["head"], "a".repeat(40));
        assert_eq!(d["scope"][0], "a/**");
        assert_eq!(d["needs_proof"], true, "no tier is the code tier");
        assert_eq!(d["id"], key("tok"));
    }

    #[test]
    fn the_passkey_answer_is_passed_on_as_base64url_only() {
        let body = json!({ "passkey": {
            "id": "abc_-1", "client": "eyJ0&x=1", "authenticator": "QUJD", "signature": "MEU"
        } });
        assert_eq!(
            proof_form(&body),
            "pk_id=abc_-1&pk_client=eyJ0x1&pk_authenticator=QUJD&pk_signature=MEU"
        );
    }
}

mod push {
    use crate::app::push::{gone, payload, said, token_input, token_ok};
    use crate::push::b64url_decode;

    #[test]
    fn a_notification_is_a_count_and_nothing_else() {
        assert_eq!(
            payload(3),
            serde_json::json!({
                "aps": { "alert": { "title": "pigeon", "body": "3 need you" }, "badge": 3, "sound": "default" }
            })
        );
        assert_eq!(said(1), "1 needs you");
        assert!(payload(1).get("link").is_none());
    }

    #[test]
    fn the_provider_token_names_its_key_and_team() {
        let input = token_input("KEY123", "TEAM45", 1_800_000_000);
        let (h, c) = input.split_once('.').expect("two parts");
        let h: serde_json::Value =
            serde_json::from_slice(&b64url_decode(h).expect("b64")).expect("json");
        let c: serde_json::Value =
            serde_json::from_slice(&b64url_decode(c).expect("b64")).expect("json");
        assert_eq!(
            (h["alg"].as_str(), h["kid"].as_str()),
            (Some("ES256"), Some("KEY123"))
        );
        assert_eq!(
            (c["iss"].as_str(), c["iat"].as_u64()),
            (Some("TEAM45"), Some(1_800_000_000))
        );
    }

    #[test]
    fn gone_tokens_and_token_shapes() {
        assert!(gone(410, "Unregistered"));
        assert!(gone(400, "BadDeviceToken"));
        assert!(!gone(400, "BadTopic"));
        assert!(!gone(200, ""));
        assert!(token_ok(&"ab".repeat(32)));
        assert!(!token_ok("xyz"));
        assert!(!token_ok(&"zz".repeat(32)));
    }
}

mod tracker {
    use crate::app::tracker::{ChangeIn, Mark, STOPS, SessionIn, Tracker, build, rail};
    use crate::brief::report::Tally;
    use crate::brief::{Brief, Commit, Draft, Revision};
    use crate::change::stage::{Needs, Stage};
    use crate::pool::route::RunsOn;
    use crate::sessions::{Class, Declared};

    const NOW: u64 = 1_800_000_000;

    fn stage(name: &str, needs: Needs) -> Stage {
        Stage {
            stage: name.into(),
            needs,
            waits_on: None,
            next: "go".into(),
            since: Some(NOW - 5),
        }
    }

    fn change(number: u32, session: &str, name: &str, needs: Needs) -> ChangeIn {
        ChangeIn {
            number,
            title: format!("change {number}"),
            session: Some(session.into()),
            stage: stage(name, needs),
        }
    }

    /// `c`, last moved an hour ago.
    fn old(mut c: ChangeIn) -> ChangeIn {
        c.stage.since = Some(NOW - 3600);
        c
    }

    fn session(label: &str, polled: Option<u64>, listening: bool) -> SessionIn {
        SessionIn {
            label: label.into(),
            declared: None,
            class: None,
            polled,
            listening,
        }
    }

    fn brief(id: &str, changes: Vec<u32>, title: &str) -> Brief {
        let draft = Draft {
            title: title.into(),
            adr: "adr.md".into(),
            step: "step 1".into(),
            model: Class::Sonnet,
            base: "abc".into(),
            scope: vec![],
            decisions: vec![],
            steps: vec![],
            checks: vec![],
            stop_if: vec![],
            budget: None,
            runs_on: RunsOn::Any,
            pool_usd: None,
            pool_minutes: None,
            commit: Commit::default(),
        };
        Brief {
            id: id.into(),
            planner: "planner".into(),
            revisions: vec![Revision {
                draft,
                at: NOW,
                reason: "created".into(),
            }],
            changes,
            tally: Tally::default(),
        }
    }

    fn only(t: &Tracker) -> &crate::app::tracker::Row {
        &t.sessions[0].changes[0]
    }

    #[test]
    fn every_stage_has_its_stop() {
        let table = [
            ("proposed", 0),
            ("authorized", 1),
            ("bounding", 2),
            ("conflicted", 2),
            ("restacking", 2),
            ("refused", 2),
            ("checking", 3),
            ("checking 2/5", 3),
            ("failed", 3),
            ("checked", 4),
            ("in-review", 4),
            ("escalated", 4),
            ("approved", 5),
            ("queued 2/3", 6),
            ("merging", 7),
            ("merged", 8),
            ("deploying", 9),
            ("done", 9),
            ("deploy-failed", 9),
        ];
        for (name, stop) in table {
            assert_eq!(rail(name, false).map(|r| r.0), Some(stop), "{name}");
        }
        assert_eq!(STOPS.len(), 10);
    }

    #[test]
    fn closed_and_unknown_stages_have_no_rail() {
        assert_eq!(rail("closed", false), None);
        assert_eq!(rail("sideways", true), None);
    }

    #[test]
    fn failed_stages_are_marked_failed() {
        for name in ["refused", "failed", "deploy-failed"] {
            assert_eq!(rail(name, true).map(|r| r.1), Some(Mark::Failed), "{name}");
        }
        assert_eq!(rail("done", false).map(|r| r.1), Some(Mark::Done));
    }

    #[test]
    fn held_is_approved_with_its_mark() {
        assert_eq!(rail("held", false), Some((5, Mark::Held)));
        assert_eq!(rail("held", true), Some((5, Mark::Held)));
    }

    #[test]
    fn a_change_needing_a_quiet_agent_is_stalled() {
        let t = build(
            &[session("a", Some(NOW - 3600), false)],
            &[old(change(1, "a", "authorized", Needs::Agent))],
            &[],
            NOW,
        );
        assert_eq!(only(&t).mark, Mark::Stalled);
        assert_eq!(t.counts.stalled, 1);
    }

    #[test]
    fn a_change_authorized_minutes_ago_is_not_stalled() {
        let t = build(
            &[session("a", Some(NOW - 8 * 3600), false)],
            &[change(1, "a", "authorized", Needs::Agent)],
            &[],
            NOW,
        );
        assert_ne!(only(&t).mark, Mark::Stalled);
        assert_eq!(t.counts.stalled, 0);
    }

    #[test]
    fn a_listening_agent_is_not_stalled() {
        let t = build(
            &[session("a", Some(NOW - 10), true)],
            &[change(1, "a", "authorized", Needs::Agent)],
            &[],
            NOW,
        );
        assert_eq!(only(&t).mark, Mark::Ok);
        assert_eq!(t.counts.stalled, 0);
    }

    #[test]
    fn a_change_takes_its_bound_brief_before_its_sessions() {
        let mut s = session("a", Some(NOW), true);
        s.declared = Some(Declared {
            model: Class::Sonnet,
            planner: Some("p".into()),
            brief: Some("b2".into()),
        });
        let briefs = [
            brief("b1", vec![1], "bound"),
            brief("b2", vec![], "declared"),
        ];
        let t = build(
            &[s],
            &[
                change(1, "a", "proposed", Needs::Agent),
                change(2, "a", "proposed", Needs::Agent),
            ],
            &briefs,
            NOW,
        );
        let rows = &t.sessions[0].changes;
        assert_eq!(rows[0].brief.as_ref().map(|b| b.id.as_str()), Some("b1"));
        assert_eq!(rows[1].brief.as_ref().map(|b| b.id.as_str()), Some("b2"));
        assert_eq!(t.sessions[0].role.as_deref(), Some("executor"));
    }

    #[test]
    fn a_live_session_without_changes_is_shown() {
        let t = build(&[session("a", None, false)], &[], &[], NOW);
        assert_eq!(t.sessions.len(), 1);
        assert_eq!(t.sessions[0].changes.len(), 0);
    }

    #[test]
    fn a_change_without_a_live_ticket_keeps_its_session_label() {
        let t = build(&[], &[change(1, "gone", "merged", Needs::None)], &[], NOW);
        assert_eq!(t.sessions[0].label, "gone");
        assert_eq!(t.sessions[0].model, None);
        assert_eq!(t.sessions[0].role, None);
    }

    #[test]
    fn sessions_needing_the_owner_come_first() {
        let t = build(
            &[
                session("a", Some(NOW - 7200), false),
                session("b", Some(NOW), true),
                session("c", Some(NOW), true),
            ],
            &[
                change(1, "a", "authorized", Needs::Agent),
                change(2, "c", "checked", Needs::Owner),
                change(3, "b", "merged", Needs::None),
            ],
            &[],
            NOW,
        );
        let order: Vec<&str> = t.sessions.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
    }

    #[test]
    fn counts_sum_owner_stalled_and_provider() {
        let t = build(
            &[session("a", None, false), session("b", Some(NOW), true)],
            &[
                old(change(1, "a", "authorized", Needs::Agent)),
                change(2, "b", "checked", Needs::Owner),
                change(3, "b", "checking 1/2", Needs::Provider),
                change(4, "b", "closed", Needs::None),
            ],
            &[],
            NOW,
        );
        assert_eq!(
            (t.counts.owner, t.counts.stalled, t.counts.provider),
            (1, 1, 1)
        );
    }

    #[test]
    fn the_queue_is_the_merging_change_then_each_queued_in_order() {
        let first = "https://claude.ai/code/a";
        let second = "https://claude.ai/code/b";
        let mut lone = change(9, first, "queued 2/4", Needs::Change);
        lone.session = None;
        let changes = [
            change(5, first, "queued 4/4", Needs::Change),
            change(3, second, "checking 1/2", Needs::Provider),
            change(7, first, "merging", Needs::Platform),
            change(8, second, "queued 3/4", Needs::Change),
            lone,
        ];
        let mut planner = session(first, None, false);
        planner.class = Some(Class::Fable);
        let t = build(&[planner], &changes, &[], NOW);
        let order: Vec<(u32, u32)> = t.queue.iter().map(|q| (q.position, q.number)).collect();
        assert_eq!(order, [(0, 7), (2, 9), (3, 8), (4, 5)]);
        assert_eq!(
            t.queue[0].model,
            Some(Class::Fable),
            "a lapsed ticket's class"
        );
        assert_eq!(
            t.queue[1].session, None,
            "a change with no session still queues"
        );
        assert_eq!(t.queue[2].model, None);
    }

    #[test]
    fn a_session_takes_its_lapsed_ticket_class_and_a_live_one_wins() {
        let a = "https://claude.ai/code/a";
        let mut s = session(a, None, false);
        s.class = Some(Class::Sonnet);
        let t = build(&[s.clone()], &[], &[], NOW);
        assert_eq!(t.sessions[0].model, Some(Class::Sonnet));
        assert_eq!(t.sessions[0].role.as_deref(), Some("executor"));
        s.declared = Some(Declared {
            model: Class::Fable,
            planner: None,
            brief: None,
        });
        let t = build(&[s], &[], &[], NOW);
        assert_eq!(t.sessions[0].model, Some(Class::Fable));
        assert_eq!(t.sessions[0].role.as_deref(), Some("planner"));
    }

    #[test]
    fn a_position_is_read_from_the_stage() {
        use crate::app::tracker::position;
        assert_eq!(position("merging"), Some(0));
        assert_eq!(position("queued 9/14"), Some(9));
        assert_eq!(position("queued 3"), Some(3));
        assert_eq!(position("approved"), None);
        assert_eq!(position("queued x/2"), None);
    }
}
