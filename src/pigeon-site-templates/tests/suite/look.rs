//! Each template wears the iOS app's look from the one token file; see
//! `.claude/decisions/2026-11-16-runtime-owner-pages-app-look.md`, its 2026-11-16 amendment.

/// The tokens, as pigeon-core's look serves them to the owner pages.
const TOKENS: &str = include_str!("../../../pigeon-core/src/look/tokens.css");

#[test]
fn each_templates_tokens_are_the_owner_pages() {
    for (name, _) in site_templates::all() {
        let copy = site_templates::text(name, "static/assets/css/tokens.css").expect(name);
        assert!(
            copy == TOKENS,
            "{name}: copy src/pigeon-core/src/look/tokens.css to static/assets/css/tokens.css"
        );
    }
}

#[test]
fn the_blogs_default_colour_is_pigeon_and_its_sheets_follow_main() {
    let theme = site_templates::text("blog", "theme.toml").expect("theme.toml");
    assert!(theme.contains("look = { color = \"pigeon\","), "{theme}");
    let base = site_templates::text("blog", "templates/base.html").expect("base.html");
    let at = |s: &str| base.find(s).unwrap_or_else(|| panic!("{s}"));
    let main = at("/assets/css/main.css");
    let tokens = at("/assets/css/tokens.css");
    let pigeon = at("/assets/css/pigeon.css");
    assert!(
        main < tokens && tokens < pigeon,
        "main, then tokens, then pigeon"
    );
    assert!(base.contains("color-theme-{{ config.extra.look.color }}"));
    let css = site_templates::text("blog", "static/assets/css/pigeon.css").expect("pigeon.css");
    assert!(
        !css.contains("url(") && !css.contains("@import"),
        "loads nothing"
    );
}

#[test]
fn the_architecture_page_reads_the_tokens_before_its_own_sheet() {
    let page = site_templates::text("architecture", "templates/index.html").expect("index.html");
    let tokens = page.find("assets/css/tokens.css").expect("tokens");
    let main = page.find("assets/css/main.css").expect("main");
    assert!(tokens < main);
    let css = site_templates::text("architecture", "static/assets/css/main.css").expect("main.css");
    assert!(
        css.contains("--link: var(--accent);"),
        "links are the app's accent"
    );
}
