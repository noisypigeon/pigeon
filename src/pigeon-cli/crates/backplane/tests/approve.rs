//! The approval page: every value escaped, no script, one form back to itself with the
//! owner's code; see `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.

#[macro_use]
#[path = "common/kept.rs"]
mod kept;

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
        tier: None,
        used: None,
    };
    let html = approve::page(&c, &grant, "tok\"><script>");
    // Only the passkey script, from this origin; see the 2026-11-09 approvals ADR §4.
    let ours = "<script src=\"/passkey.js\" defer></script>";
    assert_eq!(html.matches(ours).count(), 1, "{html}");
    assert!(!html.replace(ours, "").contains("<script"), "{html}");
    assert!(html.contains("feat: &lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("plan with &quot;quotes&quot; &amp; &lt;b&gt;tags&lt;/b&gt;"));
    assert!(html.contains("agent&lt;x&gt;"));
    assert!(html.contains("1111111111111111111111111111111111111111"));
    assert!(html.contains("value=\"tok&quot;&gt;&lt;script&gt;\""));
    assert!(html.contains("<form method=\"post\" action=\"/approve/7\">"));
    assert_eq!(html.matches("<form").count(), 1);
    assert_eq!(
        html.matches("<button").count(),
        2,
        "approve, and the hidden passkey button"
    );
    assert!(html.contains("data-passkey=\"prove\""));
    assert_eq!(
        html.matches("name=\"code\"").count(),
        1,
        "the page asks for the owner's code"
    );
    assert!(html.contains("autocomplete=\"one-time-code\""));
    assert!(html.contains("href=\"/approvals\""));
    assert!(approve::CSP.contains("default-src 'none'"));
    assert!(!approve::CSP.contains("script-src"));
    let scripts: Vec<&str> = approve::PROOF_CSP
        .split(';')
        .map(str::trim)
        .filter(|d| d.starts_with("script-src"))
        .collect();
    assert_eq!(
        scripts,
        ["script-src 'self'"],
        "only this origin's script, never inline"
    );
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
    assert_eq!(
        html.matches("<script").count(),
        1,
        "only the page's own push script"
    );
    assert!(html.contains("<script src=\"/approvals/push.js\"></script>"));
    assert!(approve::inbox(&[]).contains("Nothing is waiting."));
}

#[test]
fn a_confirm_grant_asks_for_no_code() {
    let mut c = Change::propose(
        8,
        "docs",
        "docs: x",
        "plan",
        vec!["docs/**".into()],
        "agent",
    );
    c.state = State::Checked;
    c.head = Some("2222222222222222222222222222222222222222".into());
    let grant = Grant {
        operation: "change.approve".into(),
        number: 8,
        head: c.head.clone(),
        expires: 1,
        tier: Some(backplane::approvals::Tier::Confirm),
        used: None,
    };
    let html = approve::page(&c, &grant, "tok");
    assert!(!html.contains("name=\"code\""), "{html}");
    assert!(html.contains("one tap approves it"), "{html}");
    assert_eq!(html.matches("<button").count(), 1);
    let coded = Grant {
        tier: Some(backplane::approvals::Tier::Code),
        ..grant.clone()
    };
    assert!(approve::page(&c, &coded, "tok").contains("name=\"code\""));
    let unknown = Grant {
        tier: None,
        ..grant
    };
    assert!(
        approve::page(&c, &unknown, "tok").contains("name=\"code\""),
        "a grant with no tier asks for the code"
    );
}

#[test]
fn the_inbox_shows_the_digest_and_offers_to_notify_this_device() {
    let html = approve::inbox_with(&[], &["waiting: <b>x</b>".to_string()]);
    assert!(html.contains("<h2>Digest</h2>"));
    assert!(html.contains("waiting: &lt;b&gt;x&lt;/b&gt;"), "escaped");
    assert!(html.contains("id=\"notify\""));
    assert!(html.contains("<script src=\"/approvals/push.js\"></script>"));
    assert!(approve::INBOX_CSP.contains("script-src 'self'"));
    assert!(approve::INBOX_CSP.contains("worker-src 'self'"));
    assert!(
        !approve::CSP.contains("script-src"),
        "other pages run no script"
    );
}

#[test]
fn the_inbox_installs_as_a_home_screen_app_for_ios_push() {
    let html = approve::inbox_with(&[], &[]);
    let head = &html[..html.find("</head>").expect("a head")];
    assert!(head.contains("<link rel=\"manifest\" href=\"/approvals/manifest.json\">"));
    assert!(head.contains("apple-mobile-web-app-capable"));
    assert!(html.contains("Add to Home Screen"));
    assert!(approve::INBOX_CSP.contains("manifest-src 'self'"));
    let m: serde_json::Value = serde_json::from_str(approve::MANIFEST).expect("JSON");
    assert_eq!(m["display"], "standalone");
    assert_eq!(m["start_url"], "/approvals");
}

#[test]
fn a_platform_plan_page_asks_for_the_token_and_an_apply_page_shows_the_changes() {
    use backplane::platforms::{Runs, Stage};
    let mut runs = Runs::default();
    let run = stepped!(runs.propose("platform/blue", "tok", 0)).expect("proposes");
    let page = backplane::approve::platform_page(&run, "tok<");
    assert!(
        page.contains("name=\"account_token\" type=\"password\""),
        "{page}"
    );
    assert!(page.contains("tok&lt;"), "the grant is escaped");
    assert!(!page.contains("name=\"decision\" value=\"refuse\""));
    stepped!(runs.grant_plan(1, "tok", "SEALED", "o", 1)).expect("grants");
    let change = serde_json::json!({ "action": "update", "address": "module.d1.<x>" });
    let run = stepped!(runs.planned(1, Ok(("p1".into(), vec![change])), "t2", 2)).expect("planned");
    assert_eq!(run.stage, Stage::Planned);
    let page = backplane::approve::platform_page(&run, "t2");
    assert!(page.contains("update module.d1.&lt;x&gt;"), "{page}");
    assert!(page.contains("value=\"refuse\""));
    assert!(!page.contains("account_token"));
    assert!(!page.contains("SEALED"));
}

// See `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.
#[test]
fn the_passkeys_page_adds_with_a_passkey_and_removes_with_one() {
    let key = |id: &str, synced| approve::Listed {
        id: id.into(),
        name: format!("<{id}>"),
        created: 1,
        used: None,
        synced,
    };
    let ours = "<script src=\"/passkey.js\" defer></script>";

    let none = approve::passkeys_page(&[], None, false);
    assert!(!none.contains("passkey:add"), "nothing to prove with yet");
    assert_eq!(none.matches("name=\"code\"").count(), 1, "{none}");

    let keys = [key("a", Some(true)), key("b", Some(false))];
    let html = approve::passkeys_page(&keys, Some("<said>"), false);
    assert_eq!(html.matches(ours).count(), 1, "{html}");
    assert!(!html.replace(ours, "").contains("<script"), "{html}");
    assert!(html.contains("&lt;a&gt;") && html.contains("&lt;said&gt;"));
    assert!(html.contains(", synced") && html.contains(", this device only"));
    assert!(html.contains("data-purpose=\"passkey:remove:a\""));
    assert!(html.contains("data-purpose=\"passkey:remove:b\""));
    assert!(html.contains("data-purpose=\"passkey:add\""));
    assert!(html.contains("name=\"add\""));
    assert_eq!(
        html.matches("name=\"code\"").count(),
        3,
        "two removals, one add"
    );

    let adding = approve::passkeys_page(&keys, None, true);
    assert!(!adding.contains("passkey:add"), "already proven");
    assert_eq!(
        adding.matches("name=\"code\"").count(),
        2,
        "the add form asks no code"
    );
    assert!(adding.contains("data-passkey=\"register\""));
}
