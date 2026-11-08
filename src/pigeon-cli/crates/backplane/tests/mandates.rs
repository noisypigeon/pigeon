//! Mandates: proposed, granted once with the code, bounded by scope, budget and expiry; see
//! `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §3.

use backplane::mandates::{Mandate, Mandates, State};

fn globs(list: &[&str]) -> Vec<String> {
    list.iter().map(|g| (*g).to_string()).collect()
}

const ADR: &str = ".claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md";

#[test]
fn a_mandate_needs_an_adr_a_scope_and_bounds_in_range() {
    let ok =
        Mandate::propose(1, ADR, "task", globs(&["tooling/**"]), false, None, None).expect("valid");
    assert_eq!((ok.budget, ok.hours, ok.state), (5, 24, State::Proposed));
    for (adr, scope, budget, hours) in [
        ("docs/x.md", vec!["tooling/**"], None, None),
        (".claude/decisions/../x.md", vec!["tooling/**"], None, None),
        (ADR, vec![], None, None),
        (ADR, vec!["/etc/**"], None, None),
        (ADR, vec!["tooling/**"], Some(11), None),
        (ADR, vec!["tooling/**"], Some(0), None),
        (ADR, vec!["tooling/**"], None, Some(73)),
    ] {
        assert!(
            Mandate::propose(1, adr, "task", globs(&scope), false, budget, hours).is_err(),
            "{adr} {scope:?} {budget:?} {hours:?}"
        );
    }
}

#[test]
fn a_change_fits_when_each_glob_lies_inside_the_mandates() {
    let m = Mandate::propose(
        1,
        ADR,
        "task",
        globs(&["src/pigeon-cli/crates/backplane/**", "CLAUDE.md"]),
        true,
        None,
        None,
    )
    .expect("valid");
    assert!(m.covers(&globs(&[
        "src/pigeon-cli/crates/backplane/src/**",
        "CLAUDE.md"
    ])));
    assert!(m.covers(&globs(&["src/pigeon-cli/crates/backplane/src/jobs.rs"])));
    assert!(
        !m.covers(&globs(&["src/pigeon-cli/crates/**"])),
        "wider than the mandate"
    );
    assert!(!m.covers(&globs(&["src/pigeon-cli/crates/cobot/**"])));
    assert!(!m.covers(&globs(&["**"])));
    assert!(!m.covers(&[]));
}

#[test]
fn a_grant_is_spent_once_and_the_mandate_lasts_its_hours_and_budget() {
    let mut all = Mandates::default();
    let m = all
        .propose(ADR, "task", globs(&["tooling/**"]), false, Some(2), Some(1))
        .expect("proposes");
    assert_eq!(m.id, 1);
    assert!(
        !all.get(1).expect("one").usable(10),
        "proposed is not usable"
    );
    all.offer(1, "tok", 10, 100).expect("offers");
    assert_eq!(all.peek("tok", 50).expect("peeks").id, 1);
    assert!(all.peek("tok", 100).is_err(), "expired");
    let granted = all.grant("tok", "owner@example.com", 50).expect("grants");
    assert_eq!(granted.state, State::Active);
    assert_eq!(granted.expires, Some(50 + 3600));
    assert_eq!(granted.granted_by.as_deref(), Some("owner@example.com"));
    assert!(all.grant("tok", "owner@example.com", 51).is_err(), "spent");
    let m = all.get(1).expect("one");
    assert!(m.usable(60));
    assert!(!m.usable(50 + 3600), "expired");
    let ended = all.end(1, "done").expect("ends");
    assert_eq!(ended.state, State::Ended);
    assert!(!all.get(1).expect("one").usable(60));
    assert!(
        all.offer(1, "again", 60, 200).is_err(),
        "an ended mandate isn't granted again"
    );
    let second = all
        .propose(ADR, "next", globs(&["tooling/**"]), false, None, None)
        .expect("proposes");
    assert_eq!(second.id, 2);
}

#[test]
fn the_grant_page_shows_the_mandate_escaped_and_asks_for_the_code() {
    let m = Mandate::propose(
        3,
        ADR,
        "do <b>this</b>",
        globs(&["tooling/**"]),
        true,
        Some(4),
        Some(12),
    )
    .expect("valid");
    let html = backplane::approve::mandate_page(&m, "tok\"x");
    assert!(!html.contains("<b>this</b>"), "{html}");
    assert!(html.contains("do &lt;b&gt;this&lt;/b&gt;"));
    assert!(html.contains("name=\"code\""));
    assert!(html.contains("action=\"/mandate/3\""));
    assert!(html.contains("value=\"tok&quot;x\""));
    assert!(html.contains("4 merges"));
    assert!(html.contains("12 hours"));
    assert!(!html.contains("<script"));
}
