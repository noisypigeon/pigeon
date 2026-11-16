//! A change's asks on the approval page and in the app; see
//! `.claude/decisions/2026-11-13-security-approvals-ask-owner-inputs.md` §4-§5.

use backplane::approve;
use backplane::changes::Change;
use backplane::core::app::approvals;
use backplane::core::change::asks;
use backplane::grants::Grant;
use serde_json::json;

const ASKS: &str = r#"[
  {"key": "pm_verification", "label": "pm-verification <code>", "help": "Proton Pass \"Info\" tab", "chars": "lower", "min": 20, "max": 40},
  {"key": "protonmail_verification", "label": "protonmail-verification code", "chars": "hex", "min": 40, "max": 40}
]"#;

fn asked() -> Change {
    let mut c = Change::propose(9, "x", "feat: x", "p", vec!["**".into()], "agent");
    c.asks = asks::parse(ASKS).expect("asks parse");
    c
}

fn grant(operation: &str) -> Grant {
    Grant {
        operation: operation.into(),
        number: 9,
        head: None,
        expires: 100,
        tier: None,
        used: None,
    }
}

#[test]
fn the_plan_page_asks_each_value_escaped_and_says_it_is_public() {
    let html = approve::page(&asked(), &grant("change.authorize"), "tok");
    assert!(html.contains("committed to the public repository. Never enter a secret."));
    assert!(html.contains("pm-verification &lt;code&gt;"), "{html}");
    assert!(html.contains("Proton Pass &quot;Info&quot; tab"), "{html}");
    assert!(
        html.contains("name=\"ask_pm_verification\" required"),
        "{html}"
    );
    assert!(html.contains("pattern=\"[a-z]{20,40}\""), "{html}");
    assert!(
        html.contains("name=\"ask_protonmail_verification\""),
        "{html}"
    );
    assert_eq!(html.matches("<form").count(), 1);

    let html = approve::page(&asked(), &grant("change.close"), "tok");
    assert!(!html.contains("ask_"), "only the plan's approval asks");
    let plain = Change::propose(9, "x", "feat: x", "p", vec!["**".into()], "agent");
    let html = approve::page(&plain, &grant("change.authorize"), "tok");
    assert!(!html.contains("Never enter a secret"));
}

#[test]
fn the_form_and_the_app_body_give_only_the_asked_answers() {
    let c = asked();
    let form = "grant=tok&operation=change.authorize&ask_pm_verification=+yulfrkphygclvbkaipymgmyhstrxuj+&ask_other=x";
    let got = approve::answers_from(&c, form);
    assert_eq!(got.len(), 1);
    assert_eq!(got["pm_verification"], "yulfrkphygclvbkaipymgmyhstrxuj");
    assert!(
        asks::check(&c.asks, &got)
            .expect_err("one missing")
            .contains("`protonmail_verification` is not answered")
    );

    let body = json!({ "answers": {
        "pm_verification": "yulfrkphygclvbkaipymgmyhstrxuj",
        "protonmail_verification": "e9457a054010bbbb12cc4abfce39d67198e9abd1",
        "smuggled": "x",
    }});
    let got = approvals::answers(&c, &body);
    assert_eq!(got.len(), 2, "unasked keys are dropped");
    assert!(asks::check(&c.asks, &got).is_ok());
}

#[test]
fn the_apps_detail_lists_the_asks_for_the_plans_approval_only() {
    let waiting = |operation: &str| approve::Waiting {
        number: 9,
        operation: operation.into(),
        title: "feat: x".into(),
        path: "/approve/9?grant=tok".into(),
        expires: 100,
    };
    let d = approvals::detail(
        &waiting("change.authorize"),
        &asked(),
        &grant("change.authorize"),
    );
    assert_eq!(d["asks"][0]["key"], "pm_verification");
    assert_eq!(d["asks"][0]["chars"], "lower");
    assert_eq!(d["asks"][1]["min"], 40);
    let d = approvals::detail(
        &waiting("change.approve"),
        &asked(),
        &grant("change.approve"),
    );
    assert_eq!(d["asks"], json!([]));
}
