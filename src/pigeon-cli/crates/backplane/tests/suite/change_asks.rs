//! A change's asks, answered only at its plan approval; see
//! `.claude/decisions/2026-11-13-security-approvals-ask-owner-inputs.md`.

use backplane::changes::{self, Change, Event, State};
use backplane::control::{self, Routed};
use backplane::core::change::asks::{self, Chars};
use backplane::core::change::stage::{self, Context, Needs};
use backplane::wire::Request;
use serde_json::json;

const ASKS: &str = r#"[
  {"key": "protonmail_verification", "label": "protonmail-verification code", "chars": "hex", "min": 40, "max": 40},
  {"key": "pm_verification", "label": "pm-verification code", "help": "Proton Pass → Aliases", "chars": "lower", "min": 20, "max": 40}
]"#;
const HEX: &str = "e9457a054010bbbb12cc4abfce39d67198e9abd1";
const LOWER: &str = "yulfrkphygclvbkaipymgmyhstrxuj";

fn request(op: &str, args: &serde_json::Value) -> Request {
    Request {
        operation: op.into(),
        args: args.as_object().cloned().unwrap_or_default(),
    }
}

fn asked() -> Change {
    let mut c = Change::propose(1, "x", "feat: x", "p", vec!["**".into()], "agent");
    c.asks = asks::parse(ASKS).expect("asks parse");
    c
}

/// Steps `c` by `e`: the next change, or the refusal's text.
fn on(c: &Change, e: Event) -> Result<Change, String> {
    changes::step(c, e).map(|(c, _)| c).map_err(|r| r.0)
}

fn answers(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn a_proposal_carries_its_asks_and_refuses_bad_ones() {
    let routed = control::route(
        &request(
            "change.propose",
            &json!({ "slug": "x", "title": "feat: x", "plan": "p", "scope": "a", "asks": ASKS }),
        ),
        "agent",
        1,
    );
    let Ok(Routed::Propose { asks, .. }) = routed else {
        panic!("{routed:?}");
    };
    assert_eq!(asks.len(), 2);
    assert_eq!(asks[0].chars, Chars::Hex);
    assert_eq!(asks[1].pattern(), "[a-z]{20,40}");
    assert_eq!(asks::parse("").expect("none"), Vec::new());

    let nine = format!(
        "[{}]",
        (0..9)
            .map(|i| format!(r#"{{"key": "k{i}", "label": "l"}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    for (text, want) in [
        (r#"[{"key": "Bad", "label": "l"}]"#, "lowercase letters"),
        (r#"[{"key": "k", "label": ""}]"#, "label must be"),
        (
            r#"[{"key": "k", "label": "l"}, {"key": "k", "label": "l"}]"#,
            "asked twice",
        ),
        (
            r#"[{"key": "k", "label": "l", "min": 5, "max": 2}]"#,
            "min ≤ max",
        ),
        (r#"[{"key": "k", "label": "l", "max": 999}]"#, "min ≤ max"),
        (
            r#"[{"key": "k", "label": "l", "chars": "any"}]"#,
            "not a list of asks",
        ),
        (
            r#"[{"key": "k", "label": "l", "secret": true}]"#,
            "not a list of asks",
        ),
        (nine.as_str(), "at most 8"),
    ] {
        let got = asks::parse(text).expect_err(text);
        assert!(got.contains(want), "{text}: {got}");
    }
}

#[test]
fn the_plan_is_authorized_only_with_every_answer_in_its_set() {
    let plain = on(
        &asked(),
        Event::ApprovePlan {
            by: "owner".into(),
            at: 1,
        },
    );
    assert!(
        plain
            .expect_err("unanswered")
            .contains("`protonmail_verification` is not answered")
    );

    let approve = |a| {
        on(
            &asked(),
            Event::ApprovePlanAnswered {
                by: "owner".into(),
                at: 1,
                answers: a,
            },
        )
    };
    let bad = "NOT-HEX-AND-NOT-SECRET-BUT-ECHOED-NEVER-0";
    let why = approve(answers(&[
        ("protonmail_verification", bad),
        ("pm_verification", LOWER),
    ]))
    .expect_err("off its set");
    assert!(
        why.contains("`protonmail_verification` must be 40 to 40"),
        "{why}"
    );
    assert!(!why.contains(bad), "{why}");
    let why = approve(answers(&[
        ("protonmail_verification", HEX),
        ("pm_verification", LOWER),
        ("extra", "x"),
    ]))
    .expect_err("extra");
    assert!(why.contains("`extra` was not asked"), "{why}");

    let c = approve(answers(&[
        ("protonmail_verification", HEX),
        ("pm_verification", LOWER),
    ]))
    .expect("answered");
    assert_eq!(c.state, State::Planned);
    assert_eq!(c.answers["pm_verification"], LOWER);
    let shown = serde_json::to_value(&c).expect("json");
    assert_eq!(shown["answers"]["protonmail_verification"], HEX);

    let unasked = Change::propose(1, "x", "feat: x", "p", vec!["**".into()], "agent");
    let why = on(
        &unasked,
        Event::ApprovePlanAnswered {
            by: "owner".into(),
            at: 1,
            answers: answers(&[("k", "v")]),
        },
    )
    .expect_err("nothing asked");
    assert!(why.contains("`k` was not asked"), "{why}");
}

#[test]
fn an_authorize_routes_its_answers_and_the_stage_waits_for_the_owner() {
    let r = control::route(
        &request(
            "change.authorize",
            &json!({ "number": 1, "answers": { "pm_verification": LOWER } }),
        ),
        "owner",
        5,
    );
    assert!(
        matches!(&r, Ok(Routed::Event { event: Event::ApprovePlanAnswered { answers, .. }, .. }) if answers["pm_verification"] == LOWER),
        "{r:?}"
    );
    let r = control::route(
        &request("change.authorize", &json!({ "number": 1 })),
        "owner",
        5,
    );
    assert!(
        matches!(
            &r,
            Ok(Routed::Event {
                event: Event::ApprovePlan { .. },
                ..
            })
        ),
        "{r:?}"
    );

    let cx = Context {
        covered: Some("window:2".into()),
        ..Context::default()
    };
    let s = stage::of(&asked(), &cx);
    assert_eq!(s.needs, Needs::Owner);
    assert_eq!(s.next, "answer its asks and authorize the plan");
}
