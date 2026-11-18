//! The owner pages as data; see the ADR's 2026-11-18 amendment §2.

use super::*;
use serde_json::{Value, json};

fn labels(v: &Value) -> Vec<&str> {
    v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect()
}

fn keys(v: &Value) -> Vec<(&str, &str)> {
    v["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap(), i["input"].as_str().unwrap()))
        .collect()
}

fn a_change() -> Change {
    serde_json::from_value(json!({
        "number": 7, "slug": "s", "title": "feat: t", "plan": "p", "scope": ["a/**", "b"],
        "proposer": "me", "state": "planned",
        "jobs": [{"name": "bound", "status": "queued"}],
        "violations": ["v1", "v2"],
        "asks": [{"key": "name", "label": "Name", "help": "Where"}],
        "tiers": {
            "authorize": {"tier": "code", "rule": "r1"},
            "merge": {"tier": "confirm", "rule": "r2"},
            "close": {"tier": "auto", "rule": "r3"}
        }
    }))
    .unwrap()
}

fn a_grant(operation: &str, tier: Option<&str>) -> Grant {
    serde_json::from_value(json!({
        "operation": operation, "number": 7, "head": "abc", "expires": 1, "tier": tier,
    }))
    .unwrap()
}

#[test]
fn change_authorize_asks_then_review() {
    let v = change(&a_change(), &a_grant("change.authorize", Some("code")));
    assert_eq!(v["title"], "Approve the plan — change 7");
    assert_eq!(v["body"], "feat: t");
    assert_eq!(v["verb"], "Approve the plan");
    assert_eq!(
        labels(&v),
        [
            "Branch",
            "State",
            "Proposed by",
            "Checks",
            "Violations",
            "Scope",
            "Plan",
            "Tiers"
        ]
    );
    assert_eq!(keys(&v), [("ask_name", "text"), ("review", "toggle")]);
    assert_eq!(v["inputs"][0]["help"], "Where");
    assert_eq!(v["refuse"], false);
    assert_eq!(v["change"], 7);
    assert_eq!(v["fields"][3]["value"]["text"], "bound — queued");
    assert_eq!(
        v["fields"][7]["value"]["text"],
        "authorize code (r1); merge confirm (r2); close auto (r3)"
    );
}

#[test]
fn change_close_has_no_review() {
    let v = change(&a_change(), &a_grant("change.close", None));
    assert_eq!(v["verb"], "Close unmerged");
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
}

#[test]
fn change_merge_at_confirm_has_no_inputs() {
    let v = change(&a_change(), &a_grant("change.merge", Some("confirm")));
    assert_eq!(v["verb"], "Merge");
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
}

#[test]
fn mandate_page() {
    let m: Mandate = serde_json::from_value(json!({
        "id": 3, "adr": "a.md", "summary": "do it", "scope": ["x", "y"], "may_deploy": true,
        "budget": 5, "used": 2, "hours": 8, "state": "proposed",
    }))
    .unwrap();
    let v = mandate(&m);
    assert_eq!(v["title"], "Grant mandate 3");
    assert_eq!(v["body"], "do it");
    assert_eq!(v["verb"], "Grant the mandate");
    assert_eq!(
        labels(&v),
        ["ADR", "Scope", "May deploy", "Budget", "Lasts"]
    );
    assert_eq!(v["fields"][3]["value"]["text"], "5 merges (2 used)");
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
    assert_eq!(v["refuse"], false);
    assert!(v["change"].is_null());
}

#[test]
fn lane_page() {
    let l: Lane = serde_json::from_value(json!({
        "id": 2, "reason": "rush", "line": {"changes": [4, 5]}, "minutes": 30,
        "asked_by": "me", "extends": 1, "state": "offered",
    }))
    .unwrap();
    let v = lane(&l);
    assert_eq!(v["title"], "Open fast lane 2");
    assert_eq!(v["verb"], "Open the fast lane");
    assert_eq!(labels(&v), ["Line of work", "Lasts", "Widens", "Asked by"]);
    assert_eq!(v["fields"][0]["value"]["text"], "changes !4, !5");
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
    assert_eq!(v["refuse"], false);
}

#[test]
fn grant_page() {
    use crate::minting::{Kind, Offer};
    let o = Offer {
        id: 1,
        name: "g".into(),
        kind: Kind::Rotate,
        spec: None,
        token: "t".into(),
        at: 0,
        expires: 1,
    };
    let v = grant(&o, None);
    assert_eq!(v["title"], "Rotate grant `g`");
    assert_eq!(v["verb"], "Approve");
    assert_eq!(labels(&v), ["Spec"]);
    assert_eq!(
        v["fields"][0]["value"]["text"],
        "(not in the deployed ledger)"
    );
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
    assert_eq!(v["refuse"], false);
}

#[test]
fn ci_window_page() {
    use crate::ci_window::Offer;
    let open = ci_window(&Offer {
        id: 1,
        hours: 4,
        expires: 1,
    });
    assert_eq!(open["title"], "Code checks on Cloudflare for 4 hours");
    assert_eq!(open["verb"], "Approve");
    assert_eq!(labels(&open), Vec::<&str>::new());
    let close = ci_window(&Offer {
        id: 1,
        hours: 0,
        expires: 1,
    });
    assert_eq!(close["title"], "Close the CI window");
    assert_eq!(keys(&close), Vec::<(&str, &str)>::new());
    assert_eq!(close["refuse"], false);
}

#[test]
fn pool_resume_page() {
    let v = pool_resume(&crate::pool::resume::Offer {
        id: 1,
        pool: "main".into(),
        expires: 1,
    });
    assert_eq!(v["title"], "Resume the main pool");
    assert_eq!(v["verb"], "Approve");
    assert_eq!(labels(&v), Vec::<&str>::new());
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
    assert_eq!(v["refuse"], false);
}

#[test]
fn platform_stages() {
    use crate::minting::Wanted;
    use crate::platform::platforms::{Run, Stage};
    let wanted = vec![Wanted {
        name: "r2.write".into(),
        permissions: vec!["a".into(), "b".into()],
        scope: "account".into(),
    }];
    let run = Run {
        id: 9,
        unit: "platform/blue".into(),
        ..Run::default()
    };

    let minting = platform(&run, &Ok(wanted.clone()), true);
    assert_eq!(minting["title"], "Plan platform/blue");
    assert_eq!(minting["body"], "Platform run 9.");
    assert_eq!(minting["verb"], "Mint and plan");
    assert_eq!(labels(&minting), ["r2.write"]);
    assert_eq!(minting["fields"][0]["value"]["text"], "a, b (account)");
    assert_eq!(keys(&minting), [("account_token", "secret")]);
    assert_eq!(minting["inputs"][0]["required"], false);
    assert_eq!(minting["refuse"], false);

    let pasted = platform(&run, &Err("none".into()), true);
    assert_eq!(pasted["verb"], "Plan it");
    assert_eq!(labels(&pasted), ["Permissions"]);
    assert_eq!(pasted["fields"][0]["value"]["text"], "none listed: none");
    assert_eq!(pasted["inputs"][0]["required"], true);
    assert!(pasted["inputs"][0].get("help").is_none());
    assert_eq!(pasted["refuse"], false);

    let applying = Run {
        stage: Stage::Planned,
        plan: Some("pl".into()),
        sha: Some("abc".into()),
        changes: vec![json!({"action": "create", "address": "x.y"})],
        ..run
    };
    let v = platform(&applying, &Ok(wanted), true);
    assert_eq!(v["title"], "Apply the plan of platform/blue");
    assert_eq!(v["verb"], "Apply");
    assert_eq!(labels(&v), ["Plan", "Commit", "Changes"]);
    assert_eq!(v["fields"][2]["value"]["text"], "create x.y");
    assert_eq!(keys(&v), Vec::<(&str, &str)>::new());
    assert_eq!(v["refuse"], true);
}
