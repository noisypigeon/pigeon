//! The follower's classification, steps and lines, and the hooks' renderings, without a network.

use super::{
    Answer, BACKOFF_MAX, Cursor, EXIT_REFUSED, Effect, Event, Follow, GIVE_UP, HOLD, HOLD_MIN,
    Outcome, Reply, Status, backoff, brief, classify, event_line, path, statusline, step, stop,
};

fn reply(status: u16, content_type: &str, cloudflare: bool, body: &str) -> Outcome {
    classify(&Reply {
        status,
        content_type,
        cloudflare,
        body,
    })
}

fn event(json: &str) -> Event {
    serde_json::from_str(json).expect("an event")
}

fn answer(last: u64, events: &[&str]) -> Outcome {
    Outcome::Answer(Answer {
        last,
        events: events.iter().map(|e| event(e)).collect(),
    })
}

const PLANNED: &str = r#"{"id":12533,"at":1,"kind":"change.planned","change":372,"session":"s","summary":"!372 docs(adr): x: planned","stage":{"stage":"authorized","needs":"agent","next":"push its branch","since":1}}"#;
const DEPLOY: &str =
    r#"{"id":12540,"at":2,"kind":"deploy.failed","summary":"deploy of abc failed","extra":true}"#;

#[test]
fn replies_are_classified_as_the_table_says() {
    let proxy = "upstream request failed";
    for (status, ct, cf, body, want) in [
        (
            200,
            "application/json",
            true,
            r#"{"last":7,"events":[]}"#,
            "answer",
        ),
        (502, "text/plain; charset=utf-8", false, proxy, "cut"),
        (502, "text/html", true, "<html>bad gateway</html>", "server"),
        (502, "text/plain; charset=utf-8", true, proxy, "server"),
        (503, "text/plain", false, "unavailable", "server"),
        (401, "text/plain", false, "unauthorized", "refused"),
        (403, "text/plain", true, "forbidden", "refused"),
        (404, "text/plain", true, "not found", "refused"),
        (
            200,
            "text/html",
            true,
            "<html>sign in</html>",
            "unreachable",
        ),
        (
            200,
            "application/json",
            true,
            r#"{"ok":true}"#,
            "unreachable",
        ),
    ] {
        let got = match reply(status, ct, cf, body) {
            Outcome::Answer(_) => "answer",
            Outcome::Cut => "cut",
            Outcome::Refused(_) => "refused",
            Outcome::Server(_) => "server",
            Outcome::Unreachable(_) => "unreachable",
        };
        assert_eq!(got, want, "{status} {ct} cf={cf} {body}");
    }
}

#[test]
fn an_empty_answer_is_no_events_not_a_failure() {
    let o = reply(200, "application/json", true, r#"{"last":9,"events":[]}"#);
    assert_eq!(o, answer(9, &[]));
}

#[test]
fn lines_match_session_py() {
    assert_eq!(
        event_line(&event(PLANNED)),
        "12533 !372 authorized needs=agent change.planned: !372 docs(adr): x: planned"
    );
    assert_eq!(
        event_line(&event(DEPLOY)),
        "12540 deploy.failed: deploy of abc failed"
    );
}

#[test]
fn the_path_quotes_the_label_and_names_the_hold() {
    assert_eq!(
        path("https://claude.ai/code/session_1", Cursor::Session, 20),
        "/v0/events?session=https%3A%2F%2Fclaude.ai%2Fcode%2Fsession_1&after=session&wake=agent&seconds=20"
    );
    assert_eq!(
        path("a b", Cursor::At(42), 10),
        "/v0/events?session=a%20b&after=42&wake=agent&seconds=10"
    );
}

#[test]
fn backoff_stays_between_one_second_and_a_minute() {
    for failures in [0, 1, 2, 5, 6, 30, u32::MAX] {
        for random in [0, 1, 7, 59, 60, u32::MAX] {
            let b = backoff(failures, random);
            assert!((1..=BACKOFF_MAX).contains(&b), "{failures} {random}: {b}");
        }
    }
    assert!(backoff(1, u32::MAX) <= 2);
}

#[test]
fn an_answer_prints_its_events_moves_the_cursor_and_refreshes_the_status() {
    let (s, fx) = step(
        &Follow::default(),
        answer(12540, &[PLANNED, DEPLOY]),
        100,
        0,
    );
    assert_eq!(s.cursor, Cursor::At(12540));
    assert_eq!(s.status_at, 100);
    assert_eq!(
        fx,
        vec![
            Effect::Print(event_line(&event(PLANNED))),
            Effect::Print(event_line(&event(DEPLOY))),
            Effect::Status,
        ]
    );
}

#[test]
fn a_quiet_answer_starts_from_last_and_refreshes_the_status_only_each_minute() {
    let (s, fx) = step(&Follow::default(), answer(50, &[]), 100, 0);
    assert_eq!(s.cursor, Cursor::At(50));
    assert_eq!(fx, vec![Effect::Status]);
    let (s, fx) = step(&s, answer(51, &[]), 130, 0);
    assert_eq!(
        s.cursor,
        Cursor::At(50),
        "a numbered cursor stays on what it printed"
    );
    assert_eq!(fx, vec![]);
    let (_, fx) = step(&s, answer(51, &[]), 160, 0);
    assert_eq!(fx, vec![Effect::Status]);
}

#[test]
fn a_cut_halves_the_hold_down_to_five_then_counts_as_a_failure() {
    let (s, fx) = step(&Follow::default(), Outcome::Cut, 0, 0);
    assert_eq!(s.hold, HOLD / 2);
    assert_eq!(
        fx,
        vec![Effect::Print(
            "events: the proxy cut a 20s poll; holding 10s".into()
        )]
    );
    let (s, _) = step(&s, Outcome::Cut, 0, 0);
    assert_eq!(s.hold, HOLD_MIN);
    let (s, fx) = step(&s, Outcome::Cut, 0, 0);
    assert_eq!(s.hold, HOLD_MIN);
    assert_eq!(s.failures, 1);
    assert!(matches!(fx.as_slice(), [Effect::Sleep(_)]));
}

#[test]
fn failures_are_said_at_the_fifth_and_every_sixtieth_then_recovery_once() {
    let fail = || Outcome::Server("500 boom".into());
    let (s, said) = (1..=125).fold((Follow::default(), vec![]), |(s, said), n| {
        let (s, fx) = step(&s, fail(), n, 3);
        let printed = fx.iter().any(|e| matches!(e, Effect::Print(_)));
        assert!(fx.iter().any(|e| matches!(e, Effect::Sleep(_))));
        (
            s,
            if printed {
                [said, vec![n]].concat()
            } else {
                said
            },
        )
    });
    assert_eq!(said, vec![5, 65, 125]);
    let (_, fx) = step(&s, fail(), 126, 3);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Print(_))));
    let (s, fx) = step(&s, answer(1, &[]), 200, 0);
    assert_eq!(fx[0], Effect::Print("events: following again".into()));
    assert_eq!(s.failures, 0);
    let (_, fx) = step(&Follow::default(), answer(1, &[]), 0, 0);
    assert!(!fx.contains(&Effect::Print("events: following again".into())));
}

#[test]
fn the_fifth_failure_names_its_class() {
    let s = Follow {
        failures: 4,
        ..Follow::default()
    };
    let (_, fx) = step(&s, Outcome::Unreachable("timed out".into()), 0, 0);
    assert_eq!(
        fx[0],
        Effect::Print(
            "events: 5 failed polls in a row (unreachable: timed out); still trying".into()
        )
    );
}

#[test]
fn refusals_alone_for_ten_minutes_stop_it_and_anything_else_resets_them() {
    let refused = || Outcome::Refused("401 unauthorized".into());
    let (s, _) = step(&Follow::default(), refused(), 1000, 0);
    assert_eq!(s.refused_since, Some(1000));
    let (s, fx) = step(&s, refused(), 1000 + GIVE_UP - 1, 0);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Exit(_))));
    let (reset, _) = step(&s, Outcome::Server("500".into()), 1000 + GIVE_UP - 1, 0);
    assert_eq!(reset.refused_since, None);
    let (_, fx) = step(&s, refused(), 1000 + GIVE_UP, 0);
    assert_eq!(
        fx,
        vec![
            Effect::Print("events: refused for 10 minutes (401 unauthorized); stopping".into()),
            Effect::Exit(EXIT_REFUSED),
        ]
    );
}

fn status(json: &str) -> Status {
    serde_json::from_str(json).expect("a status")
}

const STATUS: &str = r#"{"changes":[
  {"number":1,"title":"one","stage":{"stage":"queued 2/3","needs":"change","waits_on":9,"next":"!9 merges"}},
  {"number":2,"title":"two","stage":{"stage":"failed","needs":"agent","next":"fix clippy"}}
 ],
 "tasks":[{"id":4,"kind":"reconcile","change":2,"note":"rebase"}],
 "events":{"after":1,"listening":false},"session":"s","at":1000}"#;

#[test]
fn the_status_line_lists_stages_listening_and_staleness() {
    assert_eq!(
        statusline(&status(STATUS), 1000),
        "!1 queued 2/3 · !2 failed · not listening"
    );
    assert_eq!(
        statusline(&status(STATUS), 1000 + 601),
        "!1 queued 2/3 · !2 failed · not listening · stale"
    );
    assert_eq!(
        statusline(&status(r#"{"events":{"listening":true},"at":5}"#), 5),
        "listening"
    );
}

#[test]
fn the_brief_puts_what_needs_the_agent_first() {
    assert_eq!(
        brief("s", Some(&status(STATUS))),
        "pigeon session s:
- !2 two: failed, needs agent: fix clippy
- !1 one: queued 2/3, needs change: !9 merges
- task 4 (reconcile) on !2: rebase
- not following events: arm Monitor with `sh tooling/session/session.sh follow`"
    );
    assert_eq!(
        brief("s", Some(&status(r#"{"events":{"listening":true}}"#))),
        "pigeon session s:\n- no open changes"
    );
    assert!(brief("s", None).contains("didn't answer"));
}

#[test]
fn stop_reminds_only_when_a_change_needs_the_agent_and_nothing_listens() {
    let said = stop(&status(STATUS)).expect("a reminder");
    let v: serde_json::Value = serde_json::from_str(&said).expect("JSON");
    assert_eq!(
        v["systemMessage"],
        "pigeon: !2 (fix clippy) need this session, and nothing follows its events."
    );
    let listening = STATUS.replace(r#""listening":false"#, r#""listening":true"#);
    assert_eq!(stop(&status(&listening)), None);
    assert_eq!(stop(&status(r#"{"changes":[]}"#)), None);
}
