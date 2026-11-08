//! The approval page: every value escaped, no script, one form back to itself with the
//! owner's code; see `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.

use backplane::approve;
use backplane::changes::{Change, State};
use backplane::grants::Grant;

#[test]
fn values_are_escaped_for_text_and_attributes() {
    assert_eq!(
        approve::escape(r#"<a href="x" onclick='y'>&</a>"#),
        "&lt;a href=&quot;x&quot; onclick=&#39;y&#39;&gt;&amp;&lt;/a&gt;"
    );
    assert_eq!(approve::escape("plain ✓"), "plain ✓");
}

#[test]
fn the_page_shows_the_facts_escaped_and_runs_nothing() {
    let mut c = Change::propose(
        7,
        "approvals",
        "feat: <script>alert(1)</script>",
        "plan with \"quotes\" & <b>tags</b>",
        vec!["src/**".into()],
        "agent<x>",
    );
    c.state = State::Checked;
    c.head = Some("1111111111111111111111111111111111111111".into());
    let grant = Grant {
        operation: "change.approve".into(),
        number: 7,
        head: c.head.clone(),
        expires: 1,
    };
    let html = approve::page(&c, &grant, "tok\"><script>");
    assert!(!html.contains("<script"), "{html}");
    assert!(html.contains("feat: &lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("plan with &quot;quotes&quot; &amp; &lt;b&gt;tags&lt;/b&gt;"));
    assert!(html.contains("agent&lt;x&gt;"));
    assert!(html.contains("1111111111111111111111111111111111111111"));
    assert!(html.contains("value=\"tok&quot;&gt;&lt;script&gt;\""));
    assert!(html.contains("<form method=\"post\" action=\"/approve/7\">"));
    assert_eq!(html.matches("<form").count(), 1);
    assert_eq!(html.matches("<button").count(), 1);
    assert_eq!(
        html.matches("name=\"code\"").count(),
        1,
        "the page asks for the owner's code"
    );
    assert!(html.contains("autocomplete=\"one-time-code\""));
    assert!(html.contains("href=\"/approvals\""));
    assert!(approve::CSP.contains("default-src 'none'"));
    assert!(!approve::CSP.contains("script-src"));
    let done = approve::outcome("Done <x>", "by <y>");
    assert!(!done.contains("<x>") && !done.contains("<y>"));
}

#[test]
fn form_fields_decode_and_origins_match_exactly() {
    let form = "grant=ab%2Bc+d&operation=change.approve&x";
    assert_eq!(approve::field(form, "grant").as_deref(), Some("ab+c d"));
    assert_eq!(
        approve::field(form, "operation").as_deref(),
        Some("change.approve")
    );
    assert_eq!(approve::field(form, "x").as_deref(), Some(""));
    assert_eq!(approve::field(form, "missing"), None);
    assert_eq!(approve::decode("%zz%4"), "%zz%4");
    let host = "h.example";
    assert!(approve::same_origin(Some("https://h.example"), None, host));
    assert!(approve::same_origin(
        Some("https://h.example"),
        Some("same-origin"),
        host
    ));
    assert!(
        approve::same_origin(Some("null"), Some("same-origin"), host),
        "a browser sends Origin: null under some referrer policies"
    );
    assert!(approve::same_origin(None, Some("same-origin"), host));
    for (origin, site) in [
        (None, None),
        (Some("null"), None),
        (Some("null"), Some("cross-site")),
        (None, Some("same-site")),
        (Some("http://h.example"), Some("same-origin")),
        (Some("https://h.example.evil"), Some("same-origin")),
    ] {
        assert!(
            !approve::same_origin(origin, site, host),
            "{origin:?} {site:?}"
        );
    }
}

#[test]
fn dark_mode_comes_after_the_light_rules_it_overrides() {
    let html = approve::outcome("t", "m");
    let light = html.find("code,pre{background:#f2f2f2").expect("light");
    let dark = html
        .find("@media (prefers-color-scheme:dark)")
        .expect("dark");
    assert!(dark > light, "the later rule wins, so dark mode comes last");
}

#[test]
fn the_inbox_links_each_waiting_approval_escaped() {
    let waiting = vec![
        approve::Waiting {
            number: 3,
            operation: "change.merge".into(),
            title: "feat: <b>x</b>".into(),
            path: "/approve/3?grant=a\"b".into(),
            expires: 10,
        },
        approve::Waiting {
            number: 4,
            operation: "change.authorize".into(),
            title: "docs: y".into(),
            path: "/approve/4?grant=c".into(),
            expires: 10,
        },
    ];
    let html = approve::inbox(&waiting);
    assert!(!html.contains("<b>x</b>"), "{html}");
    assert!(
        html.contains("href=\"/approve/3?grant=a&quot;b\""),
        "{html}"
    );
    assert!(
        html.find("change 4").expect("4") < html.find("change 3").expect("3"),
        "newest first"
    );
    assert!(!html.contains("<script"));
    assert!(approve::inbox(&[]).contains("Nothing is waiting."));
}
