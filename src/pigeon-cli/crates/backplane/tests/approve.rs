//! The approval page: every value escaped, no script, one form back to itself.

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
    assert!(approve::same_origin(Some("https://h.example"), "h.example"));
    for bad in [
        None,
        Some("http://h.example"),
        Some("https://h.example.evil"),
        Some("null"),
    ] {
        assert!(!approve::same_origin(bad, "h.example"), "{bad:?}");
    }
}
