use super::*;

fn draft(model: Class) -> Draft {
    Draft {
        title: "Class::covers".into(),
        adr: ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md".into(),
        step: "1".into(),
        model,
        base: "7bb3087".into(),
        scope: vec!["src/a.rs".into()],
        decisions: vec!["one".into()],
        steps: vec![Step {
            act: "Add `f` to a.rs.".into(),
            files: vec!["src/a.rs".into()],
            content: Some("pub fn f() {}".into()),
            done_when: "`cargo test` passes".into(),
        }],
        checks: vec!["cargo test".into()],
        stop_if: vec!["a check fails twice".into()],
        budget: None,
        runs_on: RunsOn::Any,
        pool_usd: None,
        pool_minutes: None,
        commit: Commit {
            title: "feat(core): add f".into(),
            plan: "Adds f.".into(),
        },
    }
}

fn has(problems: &[String], text: &str) -> bool {
    problems.iter().any(|p| p == text)
}

#[test]
fn a_plain_draft_is_valid() {
    assert_eq!(validate(&draft(Class::Sonnet)), Vec::<String>::new());
}

#[test]
fn an_old_brief_parses_without_the_pool_fields() {
    let mut v = serde_json::to_value(draft(Class::Sonnet)).unwrap();
    let o = v.as_object_mut().unwrap();
    for k in ["runs_on", "pool_usd", "pool_minutes"] {
        o.remove(k);
    }
    let d: Draft = serde_json::from_value(v).unwrap();
    assert_eq!(d.runs_on, RunsOn::Any);
    assert_eq!(d.pool_usd, None);
    assert_eq!(d.pool_minutes, None);
}

#[test]
fn the_default_pool_fields_are_not_serialized() {
    let v = serde_json::to_value(draft(Class::Sonnet)).unwrap();
    let o = v.as_object().unwrap();
    for k in ["runs_on", "pool_usd", "pool_minutes"] {
        assert!(!o.contains_key(k), "{k}");
    }
}

#[test]
fn runs_on_parses_pool_and_session() {
    let mut v = serde_json::to_value(draft(Class::Sonnet)).unwrap();
    for (text, want) in [("pool", RunsOn::Pool), ("session", RunsOn::Session)] {
        v["runs_on"] = text.into();
        let d: Draft = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(d.runs_on, want);
    }
}

#[test]
fn pool_usd_must_be_a_dollar_amount() {
    for bad in ["abc", "0", "0.0", ""] {
        let mut d = draft(Class::Sonnet);
        d.pool_usd = Some(bad.into());
        assert!(
            has(&validate(&d), "`pool_usd` is not a dollar amount"),
            "{bad}"
        );
    }
}

#[test]
fn pool_minutes_must_not_be_zero() {
    let mut d = draft(Class::Sonnet);
    d.pool_minutes = Some(0);
    assert!(has(&validate(&d), "`pool_minutes` is zero"));
}

#[test]
fn only_haiku_and_sonnet_run_on_a_pool() {
    let want = "only a haiku or sonnet brief runs on a pool";
    let mut d = draft(Class::Opus);
    d.runs_on = RunsOn::Pool;
    assert!(has(&validate(&d), want));
    let mut d = draft(Class::Opus);
    d.pool_minutes = Some(10);
    assert!(has(&validate(&d), want));
    let mut d = draft(Class::Sonnet);
    d.runs_on = RunsOn::Pool;
    assert!(!has(&validate(&d), want));
}

#[test]
fn a_pool_budget_above_the_class_maximum_is_refused() {
    let mut d = draft(Class::Sonnet);
    d.pool_usd = Some("8".into());
    assert_eq!(validate(&d), Vec::<String>::new());
    d.pool_usd = Some("8.01".into());
    let want = crate::pool::config::config()
        .unwrap()
        .budget(Class::Sonnet, crate::pool::micros("8.01"), None)
        .unwrap_err();
    assert!(has(&validate(&d), &want));
}
