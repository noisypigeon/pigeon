//! The architecture site behind the platform: its paths, the passkey gate and its view cookie;
//! see `.claude/decisions/2026-11-11-runtime-docs-under-platform-passkey.md` §2 and §3.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::approve::escape;
use crate::push::{b64url, b64url_decode};

/// Where the platform serves the site.
pub const PREFIX: &str = "/docs";
/// The passkey challenge's purpose.
pub const PURPOSE: &str = "view:docs";
/// The view cookie's name.
pub const COOKIE: &str = "__Host-pigeon-docs";
/// How long a view cookie lives, in seconds.
pub const TTL: u64 = 12 * 60 * 60;
/// The served pages' policy; style attributes are the template's diagram layout.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' https://fonts.bunny.net; \
style-src-attr 'unsafe-inline'; font-src https://fonts.bunny.net; img-src 'self' data:; \
connect-src 'self'; manifest-src 'self'; frame-ancestors 'none'; base-uri 'none'";

/// What a path under [`PREFIX`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `/docs`: redirect to `/docs/`.
    Slash,
    /// The bucket key to read.
    Key(String),
}

/// What `path` answers, or `None` outside [`PREFIX`] or for a path that leaves it.
#[must_use]
pub fn target(path: &str) -> Option<Target> {
    if path == PREFIX {
        return Some(Target::Slash);
    }
    let rest = path.strip_prefix(PREFIX)?.strip_prefix('/')?;
    if rest
        .split('/')
        .any(|s| s == ".." || s == "." || s.contains('\\'))
    {
        return None;
    }
    if rest.is_empty() || rest.ends_with('/') {
        return Some(Target::Key(format!("{rest}index.html")));
    }
    Some(Target::Key(rest.to_string()))
}

fn mac(key: &[u8], subject: &str, expires: u64) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    m.update(format!("docs-view|{subject}|{expires}").as_bytes());
    m
}

/// A view cookie's value for `subject`, living until `expires`: `<expires>.<mac>`, base64url.
#[must_use]
pub fn sign(key: &[u8], subject: &str, expires: u64) -> String {
    let tag = mac(key, subject, expires).finalize().into_bytes();
    format!("{expires}.{}", b64url(&tag))
}

/// Whether `value` is a view cookie `key` signed for `subject`, living at `now`.
#[must_use]
pub fn valid(key: &[u8], value: &str, subject: &str, now: u64) -> bool {
    let Some((expires, tag)) = value.split_once('.') else {
        return false;
    };
    let (Ok(expires), Ok(tag)) = (expires.parse::<u64>(), b64url_decode(tag)) else {
        return false;
    };
    expires > now && expires <= now + TTL && mac(key, subject, expires).verify_slice(&tag).is_ok()
}

/// The view cookie's value in a `Cookie` header.
#[must_use]
pub fn cookie(header: &str) -> Option<&str> {
    header
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find_map(|(k, v)| (k == COOKIE).then_some(v))
}

/// The `Set-Cookie` header for a view cookie `value`.
#[must_use]
pub fn set_cookie(value: &str) -> String {
    format!("{COOKIE}={value}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={TTL}")
}

/// The gate: no site data, a passkey button that posts its answer to `action`, or, with no
/// passkey registered, a link to register one.
#[must_use]
pub fn gate(action: &str, has_keys: bool, said: Option<&str>) -> String {
    let said = said.map_or_else(String::new, |s| format!("<p>{}</p>", escape(s)));
    let ask = if has_keys {
        format!(
            "<form method=\"post\" action=\"{}\"><p><button type=\"button\" data-passkey=\"prove\" \
data-purpose=\"{PURPOSE}\" hidden>Prove with a passkey</button> <small data-passkey-status></small></p>\
<noscript><p>Viewing the docs needs a passkey, which needs JavaScript.</p></noscript></form>\
<script src=\"/passkey.js\" defer></script>",
            escape(action)
        )
    } else {
        "<p>No passkey is registered. <a href=\"/passkeys\">Register one</a>, then come back.</p>"
            .to_string()
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
content=\"width=device-width, initial-scale=1\"><title>pigeon docs</title></head><body \
style=\"font-family: system-ui, sans-serif; max-width: 32rem; margin: 3rem auto; padding: 0 1rem\">\
<h1>pigeon docs</h1>{said}<p>Prove it's you to see the architecture.</p>{ask}</body></html>"
    )
}

/// A file's `Content-Type`, from its extension.
#[must_use]
pub fn content_type(key: &str) -> &'static str {
    let ext = key.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
