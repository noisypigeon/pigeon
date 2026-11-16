//! The look's checks: base64, the pinned skeleton of every page, and contrast on both schemes.

use std::collections::BTreeMap;

use super::{STYLE, TOKENS, base64, page};
use crate::approval::grants::Grant;
use crate::approve;
use crate::change::{Change, State};

include!("fixtures.rs");

#[test]
fn base64_matches_the_standard_vectors() {
    let cases = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ];
    for (raw, want) in cases {
        assert_eq!(base64(raw.as_bytes()), want, "{raw}");
    }
    assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
}

#[test]
fn every_pages_forms_fields_buttons_scripts_and_links_are_pinned() {
    let want = include_str!("skeleton.txt");
    let got = skeletons();
    assert!(
        got == want,
        "a page's behaviour changed; if on purpose, pin it in look/skeleton.txt:\n{got}"
    );
}

#[test]
fn a_page_has_one_avatar_inline_and_loads_nothing() {
    let html = page("Link <this>", "", "<h1>x</h1>");
    assert!(html.contains("<title>Link &lt;this&gt;</title>"));
    assert_eq!(html.matches("<img").count(), 1);
    assert!(
        html.contains("src=\"data:image/webp;base64,UklGR"),
        "a WebP"
    );
    assert!(html.contains("<main><img class=\"avatar\""));
    assert!(
        html.contains("width=\"40\" height=\"40\""),
        "the app's 40 pt header size"
    );
    assert!(html.ends_with("<h1>x</h1></main></body></html>"));
    for css in [TOKENS, STYLE] {
        assert!(!css.contains("url("), "no image or font is fetched");
        assert!(!css.contains("@import"));
        assert!(!css.contains("</"), "stays inside its <style>");
        assert!(
            !css.contains("=\""),
            "unquoted selectors, so markup counts like `name=\"code\"` see only the markup"
        );
    }
    assert!(!html.contains("http:") && !html.contains("https:"));
}

/// The colour tokens of `css`'s block that starts at `from`, by name.
fn colours(css: &str, from: &str) -> BTreeMap<String, u32> {
    let start = css.find(from).expect("block");
    let block = &css[start..];
    let block = &block[..block.find('}').expect("end")];
    block
        .lines()
        .filter_map(|l| {
            let (k, v) = l.trim().strip_prefix("--")?.split_once(':')?;
            let hex = v.trim().strip_prefix('#')?.strip_suffix(';')?;
            Some((k.to_string(), u32::from_str_radix(hex, 16).ok()?))
        })
        .collect()
}

/// WCAG 2's relative luminance.
fn luminance(rgb: u32) -> f64 {
    let ch = |shift: u32| {
        let c = f64::from((rgb >> shift) & 0xff) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn every_text_pair_meets_aa_on_both_schemes() {
    let dark = colours(TOKENS, ":root {");
    let light = colours(TOKENS, "prefers-color-scheme: light");
    assert_eq!(dark.len(), 14, "{dark:?}");
    assert_eq!(
        dark.keys().collect::<Vec<_>>(),
        light.keys().collect::<Vec<_>>(),
        "each scheme sets every colour"
    );
    // Text on its surfaces: body text, cards, fields, chips and the accent.
    let pairs = [
        ("ink", "background"),
        ("ink2", "background"),
        ("muted", "background"),
        ("accent", "background"),
        ("ink", "card"),
        ("ink2", "card"),
        ("muted", "card"),
        ("accent", "card"),
        ("ink", "chip"),
        ("on-accent", "accent"),
    ];
    for (scheme, c) in [("dark", &dark), ("light", &light)] {
        for (fg, bg) in pairs {
            let r = contrast(c[fg], c[bg]);
            assert!(r >= 4.5, "{scheme}: {fg} on {bg} is {r:.2}:1");
        }
    }
}

#[test]
fn the_csps_add_only_inline_images_to_the_approval_pages() {
    assert_eq!(
        approve::CSP,
        "default-src 'none'; img-src data:; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
    );
    assert_eq!(
        approve::PROOF_CSP,
        "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
    );
    assert_eq!(
        approve::INBOX_CSP,
        "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; worker-src 'self'; manifest-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
    );
}

#[test]
fn every_page_wears_the_look_with_one_avatar() {
    for (name, html) in fixture_pages() {
        assert!(html.starts_with("<!doctype html>"), "{name}");
        assert!(html.contains(TOKENS) && html.contains(STYLE), "{name}");
        assert_eq!(html.matches("<img").count(), 1, "{name}");
        assert_eq!(
            html.matches("<main><img class=\"avatar\"").count(),
            1,
            "{name}"
        );
        assert!(html.ends_with("</main></body></html>"), "{name}");
    }
    let m: serde_json::Value = serde_json::from_str(approve::MANIFEST).expect("JSON");
    assert_eq!(m["background_color"], super::BACKGROUND);
}
