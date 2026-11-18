//! The presenters' frame: names, families, the check and what is still to present; see the ADR's 2026-11-17 amendment on presenting.

use super::{approval, change, checked, rest};
use crate::view::View;
use crate::wire::catalog;
use serde_json::json;

fn family(name: &str) -> &'static str {
    if change::PREFIXES.iter().any(|p| name.starts_with(p)) {
        "change"
    } else if approval::PREFIXES.iter().any(|p| name.starts_with(p)) {
        "approval"
    } else {
        "rest"
    }
}

#[test]
fn presented_names_are_unique_and_in_the_catalog() {
    let names = super::presented();
    let mut seen = std::collections::BTreeSet::new();
    for n in &names {
        assert!(seen.insert(*n), "{n} is presented twice");
        assert!(
            catalog().iter().any(|o| o.name == *n),
            "{n} is not in the catalog"
        );
    }
}

#[test]
fn every_name_is_in_its_family() {
    for (fam, names) in [
        ("change", change::NAMES),
        ("approval", approval::NAMES),
        ("rest", rest::NAMES),
    ] {
        for n in names {
            assert_eq!(family(n), fam, "{n} is in the wrong family");
        }
    }
}

#[test]
fn each_family_pins_what_it_still_has_to_present() {
    for (fam, names, pending) in [
        ("change", change::NAMES, change::PENDING),
        ("approval", approval::NAMES, approval::PENDING),
        ("rest", rest::NAMES, rest::PENDING),
    ] {
        let left = catalog()
            .iter()
            .filter(|o| family(o.name) == fam && !names.contains(&o.name))
            .count();
        assert_eq!(left, pending, "{fam} has {left} left to present");
    }
}

#[test]
fn checked_passes_none_and_a_valid_view() {
    assert_eq!(checked(View::Card, None), Ok(None));
    let v = super::card("t", None, vec![], vec![]);
    assert_eq!(checked(View::Card, Some(v.clone())), Ok(Some(v)));
}

#[test]
fn checked_refuses_a_view_of_another_kind() {
    let v = super::card("t", None, vec![], vec![]);
    let problems = checked(View::List, Some(v)).expect_err("a card is not a list");
    assert!(
        problems[0].starts_with("$.view: expected list"),
        "{problems:?}"
    );
}

#[test]
fn builders_make_valid_views() {
    use super::{action, card, code, count, detail, duration, field, link, list, progress, row};
    use super::{section, sha, text, time};
    let fields = vec![
        field("a", text("x")),
        field("b", code("y")),
        field("c", sha("abc1234")),
        field("d", time(1)),
        field("e", duration(2)),
        field("f", count(3)),
        field("g", link("l", "https://x.dev")),
        field("h", progress(1, 2, "ok")),
    ];
    let go = action(
        "Go",
        "change.status",
        json!({ "number": 1 }),
        "primary",
        false,
    );
    let c = card("t", Some("b"), fields.clone(), vec![go.clone()]);
    assert_eq!(checked(View::Card, Some(c)).map(|v| v.is_some()), Ok(true));
    let d = detail(
        "t",
        vec![section(Some("h"), fields, Some("x"))],
        vec![go.clone()],
    );
    assert_eq!(
        checked(View::Detail, Some(d)).map(|v| v.is_some()),
        Ok(true)
    );
    let l = list(
        "t",
        vec![row(
            "1",
            "one",
            Some("s"),
            Some("b"),
            Some("agent"),
            Some(go),
        )],
        Some("n"),
    );
    assert_eq!(checked(View::List, Some(l)).map(|v| v.is_some()), Ok(true));
}

#[test]
fn scalars_take_only_scalars() {
    let got = super::scalars(&json!({ "a": 1, "b": "x", "c": true, "d": [1], "e": {} }));
    let labels: Vec<_> = got
        .iter()
        .map(|f| f["label"].as_str().expect("label"))
        .collect();
    let kinds: Vec<_> = got
        .iter()
        .map(|f| f["value"]["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(labels, ["a", "b", "c"]);
    assert_eq!(kinds, ["count", "text", "text"]);
    let one = super::scalars(&json!("paused"));
    assert_eq!(one.len(), 1);
    assert_eq!(one[0]["label"], "result");
}

#[test]
fn every_declared_result_presents() {
    let presented = super::presented();
    let mut checked_count = 0;
    for o in catalog() {
        if !presented.contains(&o.name) {
            continue;
        }
        let Some(fields) = crate::wire::results::result(o.name) else {
            continue;
        };
        let r = crate::wire::Response {
            operation: o.name.into(),
            ok: true,
            result: crate::wire::results::sample(fields),
            violations: vec![],
        };
        let v = checked(o.view, super::view(o.name, &r))
            .unwrap_or_else(|p| panic!("{}: {p:?}", o.name))
            .unwrap_or_else(|| panic!("{} presented nothing", o.name));
        assert!(
            v["title"].as_str().is_some_and(|t| !t.trim().is_empty()),
            "{} has no title",
            o.name
        );
        checked_count += 1;
    }
    assert!(checked_count > 0, "no declared result was checked");
}
