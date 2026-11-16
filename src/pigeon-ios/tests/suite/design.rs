//! The web's tokens equal the app's: `look/tokens.css` mirrors `Theme.swift`; see
//! .claude/decisions/2026-11-16-runtime-owner-pages-app-look.md §1.

use std::collections::BTreeMap;

fn read(rel: &str) -> String {
    std::fs::read_to_string(pigeon_ios::root().join(rel)).expect(rel)
}

/// `camelCase` as `kebab-case`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `Theme`'s `static let <name> = color(0x<light>, 0x<dark>)`, as kebab name → (light, dark).
fn swift_colours(swift: &str) -> BTreeMap<String, (String, String)> {
    swift
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("static let ")?;
            let (name, rest) = rest.split_once(" = color(0x")?;
            let (light, rest) = rest.split_once(", 0x")?;
            let dark = rest.strip_suffix(')')?;
            Some((kebab(name), (light.to_uppercase(), dark.to_uppercase())))
        })
        .collect()
}

/// The `--<name>: #<hex>;` lines of the CSS block that starts at `from`.
fn css_colours(css: &str, from: &str) -> BTreeMap<String, String> {
    let block = &css[css.find(from).expect(from)..];
    let block = &block[..block.find('}').expect("end")];
    block
        .lines()
        .filter_map(|l| {
            let (k, v) = l.trim().strip_prefix("--")?.split_once(": #")?;
            Some((k.to_string(), v.strip_suffix(';')?.to_uppercase()))
        })
        .collect()
}

#[test]
fn the_webs_colours_radius_and_shadow_are_the_apps() {
    let swift = read("Pigeon/Theme.swift");
    let css = read("../pigeon-core/src/look/tokens.css");
    let app = swift_colours(&swift);
    assert!(app.len() >= 14, "{app:?}");
    let dark = css_colours(&css, ":root {");
    let light = css_colours(&css, "prefers-color-scheme: light");
    let web: BTreeMap<String, (String, String)> = dark
        .iter()
        .map(|(k, d)| {
            (
                k.clone(),
                (light.get(k).cloned().unwrap_or_default(), d.clone()),
            )
        })
        .collect();
    assert_eq!(
        web, app,
        "tokens.css (left) differs from Theme.swift (right)"
    );
    assert!(swift.contains("var radius: CGFloat = 14") && css.contains("--radius-card: 14px;"));
    assert!(swift.contains(".offset(x: 3, y: 3)") && css.contains("--shadow-offset: 3px;"));
}
