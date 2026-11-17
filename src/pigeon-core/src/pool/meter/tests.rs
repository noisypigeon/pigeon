#![allow(clippy::many_single_char_names)]

use super::*;
use crate::pool::anthropic::Messages;
use crate::pool::config::{Budget, config};
use crate::sessions::Class;

const NOW: u64 = 1_795_000_000;
const KEY: &[u8] = b"session-key";
const WS: &str = "wrkspc_test";

fn pool() -> Pool {
    let mut p = config()
        .expect("pool.yaml")
        .pool("anthropic")
        .expect("anthropic")
        .clone();
    p.enabled = true;
    p.cycle_day = Some(16);
    p
}

fn budget() -> Budget {
    Budget {
        usd: 4_000_000,
        fresh_tokens: 3_000_000,
        requests: 200,
        minutes: 40,
    }
}

fn led() -> Ledger {
    let p = pool();
    ledger::admit_task(
        &ledger::new(&p, NOW),
        &p,
        "b9.1",
        Class::Sonnet,
        budget(),
        NOW,
    )
    .expect("admitted")
}

fn info() -> TaskInfo {
    TaskInfo {
        label: "pool:b9.1".into(),
        change: 900,
        brief: "b9".into(),
        class: "sonnet".into(),
        model: "claude-sonnet-5-5".into(),
    }
}

fn live() -> Caller {
    caller("pool:b9.1", Ticket::Live, BTreeSet::from([900]))
}

const BODY: &str = r#"{"model":"claude-sonnet-5-5","max_tokens":1000,"stream":true,"messages":[]}"#;

fn run(
    p: &Pool,
    l: &Ledger,
    c: &Caller,
    ws: Option<&str>,
    state: ChangeState,
    body: &str,
) -> Result<Forward, Refuse> {
    let i = info();
    let v = Vet {
        gateway: &Messages,
        pool: p,
        ledger: l,
        workspace: ws,
        task: "b9.1",
        info: Some(&i),
        caller: c,
        change: state,
    };
    vet(&v, "h1", "/v1/messages", body.as_bytes(), NOW)
}

#[test]
fn a_token_round_trips_with_a_dotted_task() {
    let k = task_key(KEY);
    let t = sign_task(&k, "anthropic", "b9.1", NOW + 60);
    assert_eq!(
        verify_task(&k, &t, NOW),
        Ok(("anthropic".into(), "b9.1".into()))
    );
}

#[test]
fn a_token_is_refused_expired_forged_or_under_another_key() {
    let k = task_key(KEY);
    let t = sign_task(&k, "anthropic", "b9.1", NOW + 60);
    assert!(
        verify_task(&k, &t, NOW + 60)
            .unwrap_err()
            .contains("expired")
    );
    let forged = t.replacen("b9.1", "b9.2", 1);
    assert!(verify_task(&k, &forged, NOW).is_err());
    assert!(verify_task(&task_key(b"other"), &t, NOW).is_err());
    assert!(verify_task(&k, "sk-ant-api03-xyz", NOW).is_err());
    assert!(verify_task(&k, "", NOW).is_err());
}

#[test]
fn the_task_key_is_not_the_session_key() {
    assert_ne!(task_key(KEY), KEY.to_vec());
}

#[test]
fn a_live_task_is_reserved_with_its_metadata() {
    let f = run(&pool(), &led(), &live(), Some(WS), ChangeState::Open, BODY).expect("forward");
    assert!(f.hold > 0);
    assert!(f.stream);
    assert_eq!(f.ledger.holds["h1"].amount, f.hold);
    assert_eq!(f.meta.change, 900);
    assert_eq!(f.meta.brief, "b9");
    assert_eq!(f.meta.class, "sonnet");
    assert_eq!(f.meta.pool, "anthropic");
}

#[test]
fn a_disabled_pool_refuses_everything() {
    let mut p = pool();
    p.enabled = false;
    let r = run(&p, &led(), &live(), Some(WS), ChangeState::Open, BODY).unwrap_err();
    assert_eq!((r.status, r.why.as_str()), (403, "pool: disabled"));
}

#[test]
fn no_pinned_workspace_refuses_everything() {
    for ws in [None, Some("")] {
        let r = run(&pool(), &led(), &live(), ws, ChangeState::Open, BODY).unwrap_err();
        assert_eq!(r.status, 503);
    }
}

#[test]
fn an_unknown_task_is_refused() {
    let v = Vet {
        gateway: &Messages,
        pool: &pool(),
        ledger: &led(),
        workspace: Some(WS),
        task: "b9.1",
        info: None,
        caller: &live(),
        change: ChangeState::Open,
    };
    let r = vet(&v, "h1", "/v1/messages", BODY.as_bytes(), NOW).unwrap_err();
    assert_eq!(r.why, "pool: no task b9.1");
}

#[test]
fn an_evicted_or_closed_change_stops_the_task() {
    for s in [
        ChangeState::Evicted,
        ChangeState::Closed,
        ChangeState::Merged,
    ] {
        let r = run(&pool(), &led(), &live(), Some(WS), s, BODY).unwrap_err();
        assert_eq!(r.status, 403);
        assert!(r.why.starts_with("pool: change 900 evicted"), "{}", r.why);
        assert_eq!(r.close, Some("stopped (evicted)"));
    }
}

#[test]
fn a_dead_ticket_or_a_change_not_held_stops_the_task() {
    for t in [Ticket::Missing, Ticket::Lapsed, Ticket::Revoked] {
        let c = caller("pool:b9.1", t, BTreeSet::from([900]));
        let r = run(&pool(), &led(), &c, Some(WS), ChangeState::Open, BODY).unwrap_err();
        assert_eq!(r.close, Some("stopped (evicted)"));
    }
    let c = caller("pool:b9.1", Ticket::Live, BTreeSet::new());
    let r = run(&pool(), &led(), &c, Some(WS), ChangeState::Open, BODY).unwrap_err();
    assert!(r.why.contains("not this session's"), "{}", r.why);
}

#[test]
fn admission_refuses_another_model_and_server_tools() {
    let opus = r#"{"model":"claude-opus-5-5","max_tokens":10,"messages":[]}"#;
    let r = run(&pool(), &led(), &live(), Some(WS), ChangeState::Open, opus).unwrap_err();
    assert_eq!(r.status, 400);
    assert!(r.why.contains("is not allowed"));
    let tool = r#"{"model":"claude-sonnet-5-5","max_tokens":10,"tools":[{"type":"web_search_20250305"}],"messages":[]}"#;
    let r = run(&pool(), &led(), &live(), Some(WS), ChangeState::Open, tool).unwrap_err();
    assert!(r.why.contains("is refused"));
}

#[test]
fn the_helper_model_is_admitted() {
    let h = r#"{"model":"claude-haiku-5-5","max_tokens":10,"messages":[]}"#;
    assert!(run(&pool(), &led(), &live(), Some(WS), ChangeState::Open, h).is_ok());
}

#[test]
fn a_spent_budget_stops_the_task() {
    let big = r#"{"model":"claude-sonnet-5-5","max_tokens":64000,"messages":[]}"#;
    let mut l = led();
    let mut n = 0;
    let r = loop {
        let i = info();
        let c = live();
        let v = Vet {
            gateway: &Messages,
            pool: &pool(),
            ledger: &l,
            workspace: Some(WS),
            task: "b9.1",
            info: Some(&i),
            caller: &c,
            change: ChangeState::Open,
        };
        match vet(&v, &format!("h{n}"), "/v1/messages", big.as_bytes(), NOW) {
            Ok(f) => l = f.ledger,
            Err(r) => break r,
        }
        n += 1;
    };
    assert_eq!(r.why, "pool: task b9.1 spent its dollars");
    assert_eq!(r.close, Some("stopped"));
}

fn usage(model: &str) -> Usage {
    Usage {
        model: model.into(),
        input: 100,
        output: 50,
        complete: true,
        ..Usage::default()
    }
}

fn reserved() -> (Ledger, Micros) {
    let f = run(&pool(), &led(), &live(), Some(WS), ChangeState::Open, BODY).expect("forward");
    (f.ledger, f.hold)
}

fn seen<'a>(status: u16, account: Option<&'a str>, body: &'a str, u: Option<Usage>) -> Seen<'a> {
    Seen {
        status,
        account,
        body,
        usage: u,
    }
}

#[test]
fn a_good_response_is_charged_its_usage() {
    let (l, hold) = reserved();
    let u = usage("claude-sonnet-5-5");
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(200, Some(WS), "", Some(u.clone())),
        NOW,
    );
    let want = Messages.cost(&u, &pool());
    assert!(want < hold);
    assert_eq!(s.cost, want);
    assert_eq!(s.trips, Vec::<String>::new());
    assert!(s.ledger.holds.is_empty());
    assert_eq!(s.ledger.cycle.spent, want);
    assert_eq!(s.ledger.paused, None);
}

#[test]
fn a_broken_stream_is_charged_its_reservation() {
    let (l, hold) = reserved();
    let mut u = usage("claude-sonnet-5-5");
    u.complete = false;
    for usage in [None, Some(u)] {
        let s = settle(
            &Messages,
            &pool(),
            &l,
            "h1",
            WS,
            &seen(200, Some(WS), "", usage),
            NOW,
        );
        assert_eq!(s.cost, hold);
        assert_eq!(s.ledger.cycle.spent, hold);
    }
}

#[test]
fn another_workspace_or_none_trips() {
    let (l, _) = reserved();
    for account in [Some("wrkspc_other"), None] {
        let u = Some(usage("claude-sonnet-5-5"));
        let s = settle(
            &Messages,
            &pool(),
            &l,
            "h1",
            WS,
            &seen(200, account, "", u),
            NOW,
        );
        assert_eq!(s.trips.len(), 1);
        assert!(s.ledger.paused.is_some());
    }
}

#[test]
fn the_gateway_unreachable_is_charged_and_pauses() {
    let (l, hold) = reserved();
    for (status, body) in [
        (0, ""),
        (502, "<html>bad gateway</html>"),
        (500, r#"{"error":[{"code":2001,"message":"x"}]}"#),
    ] {
        let s = settle(
            &Messages,
            &pool(),
            &l,
            "h1",
            WS,
            &seen(status, None, body, None),
            NOW,
        );
        assert_eq!(s.cost, hold, "{status}");
        assert_eq!(s.trips.len(), 1, "{status}");
        assert!(s.ledger.paused.is_some());
    }
}

#[test]
fn no_stored_key_and_a_refused_token_pause() {
    let (l, _) = reserved();
    let nokey = r#"{"success":false,"error":[{"code":2044,"message":"no key"}]}"#;
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(400, None, nokey, None),
        NOW,
    );
    assert_eq!(
        s.trips,
        vec!["the AI Gateway has no stored key".to_string()]
    );
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(401, None, "{}", None),
        NOW,
    );
    assert_eq!(
        s.trips,
        vec!["the AI Gateway refused the token".to_string()]
    );
}

#[test]
fn the_gateway_rate_limit_passes_without_a_pause() {
    let (l, hold) = reserved();
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(429, None, "{}", None),
        NOW,
    );
    assert_eq!(s.trips, Vec::<String>::new());
    assert_eq!(s.ledger.paused, None);
    assert_eq!(s.cost, hold);
}

#[test]
fn anthropic_exhausted_early_trips_and_marks_the_pool() {
    let (l, _) = reserved();
    let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#;
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(400, Some(WS), body, None),
        NOW,
    );
    assert!(s.ledger.exhausted_until.is_some());
    assert_eq!(s.trips.len(), 1);
}

#[test]
fn an_anthropic_overload_is_no_signal() {
    let (l, _) = reserved();
    let body = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(529, Some(WS), body, None),
        NOW,
    );
    assert_eq!(s.trips, Vec::<String>::new());
}

#[test]
fn a_model_outside_the_table_is_priced_high_and_pauses() {
    let (l, _) = reserved();
    let u = usage("claude-unknown-9");
    let s = settle(
        &Messages,
        &pool(),
        &l,
        "h1",
        WS,
        &seen(200, Some(WS), "", Some(u)),
        NOW,
    );
    assert!(s.ledger.paused.is_some());
    assert!(
        s.trips
            .iter()
            .any(|t| t.contains("outside the price table"))
    );
}

#[test]
fn sse_reads_usage_across_split_chunks() {
    let stream = concat!(
        "event: message_start\r\n",
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-5-5\",\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\r\n\r\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"héllo\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":640}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    )
    .as_bytes();
    for size in [1, 3, 7, 64, stream.len()] {
        let mut s = Sse::default();
        for c in stream.chunks(size) {
            s.feed(&Messages, c);
        }
        assert_eq!(s.usage.model, "claude-sonnet-5-5", "{size}");
        assert_eq!((s.usage.input, s.usage.output), (12, 640), "{size}");
        assert!(s.usage.complete, "{size}");
    }
}

#[test]
fn sse_without_a_stop_is_incomplete() {
    let mut s = Sse::default();
    s.feed(
        &Messages,
        b"data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1}}}\n",
    );
    assert!(!s.usage.complete);
}
