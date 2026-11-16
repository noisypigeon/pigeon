// Every owner page, rendered from fixed inputs, by name; `include!`d where `approve`, `Change`,
// `State` and `Grant` are in scope.

/// A grant on change 7 for `operation` at `tier`.
fn fixture_grant(
    operation: &str,
    tier: Option<crate::approval::tiers::Tier>,
) -> Grant {
    Grant {
        operation: operation.into(),
        number: 7,
        head: Some("1111111111111111111111111111111111111111".into()),
        expires: 1,
        tier,
        used: None,
    }
}

/// Change 7, checked, with a check, a plan, asks and tiers.
fn fixture_change() -> Change {
    let mut c = Change::propose(
        7,
        "approvals",
        "feat(look): the owner pages take the app's look",
        "Per the look ADR.\n- tokens\n- the stylesheet",
        vec!["src/pigeon-core/src/look/**".into(), "src/pigeon-core/src/look.rs".into()],
        "agent",
    );
    c.state = State::Checked;
    c.head = Some("1111111111111111111111111111111111111111".into());
    c.jobs = serde_json::from_value(serde_json::json!([
        {"name": "bound", "status": "passed"},
        {"name": "clippy", "status": "running"}
    ]))
    .expect("jobs");
    c.asks = serde_json::from_value(serde_json::json!([
        {"key": "zone", "label": "Zone ID", "help": "Dashboard, Overview", "chars": "hex", "min": 32, "max": 32}
    ]))
    .expect("asks");
    c.tiers = serde_json::from_value(serde_json::json!({
        "authorize": {"tier": "code", "rule": "default"},
        "merge": {"tier": "owner", "rule": "guarded: src/pigeon-core/src/approve.rs"},
        "close": {"tier": "confirm", "rule": "default"}
    }))
    .expect("tiers");
    c
}

/// Each page's name and HTML.
fn fixture_pages() -> Vec<(&'static str, String)> {
    use crate::approval::tiers::Tier;
    let c = fixture_change();
    let waiting = vec![
        approve::Waiting {
            number: 7,
            operation: "change.approve".into(),
            title: c.title.clone(),
            path: "/approve/7?grant=tok".into(),
            expires: 1,
        },
        approve::Waiting {
            number: 3,
            operation: "mandate.grant".into(),
            title: "Restyle the owner pages".into(),
            path: "/mandate/3?grant=tok".into(),
            expires: 1,
        },
    ];
    let keys = [approve::Listed {
        id: "a1".into(),
        name: "iPhone".into(),
        created: 1,
        used: Some(2),
        synced: Some(true),
    }];
    let lacks = ["two passkeys (there is 1)".to_string()];
    let cutover = approve::Cutover {
        missing: &lacks,
        since: None,
    };
    let mandate: crate::approval::mandates::Mandate = serde_json::from_value(serde_json::json!({
        "id": 3, "adr": ".claude/decisions/2026-11-16-runtime-owner-pages-app-look.md",
        "summary": "Restyle the owner pages", "scope": ["src/pigeon-core/src/look/**"],
        "may_deploy": true, "budget": 4, "hours": 8, "state": "proposed"
    }))
    .expect("mandate");
    let offer: crate::minting::Offer = serde_json::from_value(serde_json::json!({
        "id": 2, "name": "sites-r2", "kind": "rotate", "token": "tok", "at": 1, "expires": 2
    }))
    .expect("offer");
    let window = crate::ci_window::Offer {
        id: 4,
        hours: 8,
        expires: 1,
    };
    let offered: crate::platform::platforms::Run = serde_json::from_value(serde_json::json!({
        "id": 5, "unit": "platform/blue", "stage": "offered"
    }))
    .expect("run");
    let planned: crate::platform::platforms::Run = serde_json::from_value(serde_json::json!({
        "id": 6, "unit": "platform/blue", "stage": "planned", "plan": "p1",
        "changes": [{"action": "update", "address": "module.d1.cloudflare_d1_database.main"}]
    }))
    .expect("run");
    let devices = vec![(
        "d1".to_string(),
        "Willow's iPhone".to_string(),
        1_u64,
        2_u64,
    )];
    vec![
        ("inbox", approve::inbox_with(&waiting, &["1 change waits for you".into()])),
        ("inbox-empty", approve::inbox(&[])),
        (
            "approve-authorize",
            approve::page(&c, &fixture_grant("change.authorize", Some(Tier::Code)), "tok"),
        ),
        (
            "approve-head",
            approve::page(&c, &fixture_grant("change.approve", Some(Tier::Owner)), "tok"),
        ),
        (
            "approve-close",
            approve::page(&c, &fixture_grant("change.close", Some(Tier::Code)), "tok"),
        ),
        (
            "approve-confirm",
            approve::page(&c, &fixture_grant("change.pin", Some(Tier::Confirm)), "tok"),
        ),
        ("mandate", approve::mandate_page(&mandate, "tok")),
        ("grant", approve::grant_page(&offer, None, "tok")),
        ("ci-window", approve::ci_window_page(&window, "tok")),
        ("platform-plan", approve::platform_page(&offered, "tok")),
        ("platform-apply", approve::platform_page(&planned, "tok")),
        ("link", approve::link_page("Willow's iPhone", "BASE64KEY", "ab:cd:ef:01")),
        ("devices", approve::devices_page(&devices, Some("Unlinked."))),
        ("passkeys", approve::passkeys_page(&keys, None, false, &cutover)),
        ("enroll", approve::enroll_page(None)),
        ("outcome", approve::outcome("Not done", "This link has expired.")),
    ]
}

/// The tags that carry a page's behaviour, in order, one per line: its forms, fields, buttons,
/// scripts and links (`a` and `link`), with their attributes.
fn skeleton(html: &str) -> String {
    const TAGS: [&str; 7] = ["form", "input", "button", "script", "a", "label", "link"];
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let name: String = rest
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        let end = rest.find('>').unwrap_or(rest.len());
        if TAGS.contains(&name.as_str()) {
            out.push('<');
            out.push_str(&rest[..end]);
            out.push_str(">\n");
        }
        rest = &rest[end..];
    }
    out
}

/// Every page's skeleton, under its name.
fn skeletons() -> String {
    use std::fmt::Write as _;
    fixture_pages().iter().fold(String::new(), |mut out, (name, html)| {
        let _ = write!(out, "== {name}\n{}", skeleton(html));
        out
    })
}
