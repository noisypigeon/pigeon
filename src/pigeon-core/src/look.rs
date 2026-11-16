//! The owner pages' look, the iOS app's: its tokens, one stylesheet and the avatar, inline; see
//! `.claude/decisions/2026-11-16-runtime-owner-pages-app-look.md`.

use std::sync::LazyLock;

/// The tokens, mirrored from `Theme.swift`.
pub const TOKENS: &str = include_str!("look/tokens.css");

/// The rules over the pages' plain elements.
pub const STYLE: &str = include_str!("look/style.css");

/// The avatar, a 144 px WebP of the app's `Avatar.png`.
const AVATAR: &[u8] = include_bytes!("look/avatar.webp");

/// The page background, for `theme-color` and the web app manifest.
pub const BACKGROUND: &str = "#14121A";

/// The avatar's `<img>`, its image a `data:` URI, so the CSP needs only `img-src data:`.
static AVATAR_IMG: LazyLock<String> = LazyLock::new(|| {
    format!(
        "<img class=\"avatar\" src=\"data:image/webp;base64,{}\" width=\"72\" height=\"72\" alt=\"\">",
        base64(AVATAR)
    )
});

/// Standard base64, padded.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A page titled `title` (escaped here), with `head` added to its `<head>` and `body` under the
/// avatar; `head` and `body` are markup, already escaped.
#[must_use]
pub fn page(title: &str, head: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
<meta name=\"color-scheme\" content=\"dark light\">\
<meta name=\"theme-color\" content=\"{BACKGROUND}\">{head}\
<title>{}</title><style>{TOKENS}{STYLE}</style></head>\
<body><main>{}{body}</main></body></html>",
        crate::approve::escape(title),
        *AVATAR_IMG
    )
}

#[cfg(test)]
mod tests;
