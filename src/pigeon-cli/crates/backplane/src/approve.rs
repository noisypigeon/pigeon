//! The approval page and its inbox: server-rendered HTML with no scripts, every value escaped;
//! see `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::changes::Change;
use crate::grants::Grant;

/// The page's Content-Security-Policy: no scripts, no external loads, forms only to itself.
pub const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The inbox's policy: [`CSP`], plus its own script, its fetches and its service worker.
pub const INBOX_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; worker-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// `s` escaped for HTML text and attribute values.
#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The page's field for the owner's code, at the code tier.
const CODE_FIELD: &str = "<p><label>Authenticator code<br><input name=\"code\" inputmode=\"numeric\" \
autocomplete=\"one-time-code\" pattern=\"[0-9]{6}\" maxlength=\"6\" required></label></p>";

/// What the page says instead, at the confirm tier.
const CONFIRM_NOTE: &str =
    "<p>This is a low-risk operation, so one tap approves it; no code is needed.</p>";

/// A verb for the button.
fn verb(operation: &str) -> &'static str {
    match operation {
        "change.authorize" => "Approve the plan",
        "change.approve" => "Approve this head",
        "change.merge" => "Merge",
        "change.close" => "Close unmerged",
        "mandate.grant" => "Grant the mandate",
        _ => "Confirm",
    }
}

fn shell(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{}</title><style>\
body{{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem;color:#1b1b1b;background:#fff}}\
code,pre{{background:#f2f2f2;color:inherit;border-radius:4px;padding:.1rem .3rem;font-size:.9em}}\
@media (prefers-color-scheme:dark){{body{{color:#e8e8e8;background:#141414}}code,pre{{background:#262626}}}}\
pre{{padding:.75rem;white-space:pre-wrap;overflow-wrap:anywhere}}\
dt{{font-weight:600;margin-top:.5rem}}dd{{margin:0}}\
button{{font:inherit;padding:.6rem 1.2rem;border-radius:6px;border:0;background:#2b59c3;color:#fff;cursor:pointer}}\
input{{font:inherit;font-size:1.4rem;letter-spacing:.3rem;width:9rem;padding:.4rem}}a{{color:#2b59c3}}\
</style></head><body>{body}</body></html>",
        escape(title)
    )
}

/// One approval waiting in the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    /// The change.
    pub number: u32,
    /// The operation.
    pub operation: String,
    /// The change's title.
    pub title: String,
    /// The page's path, `/approve/<number>?grant=<token>`.
    pub path: String,
    /// When its grant expires, in Unix seconds.
    pub expires: u64,
}

/// The inbox: every approval waiting, newest first, each a link to its page.
#[must_use]
pub fn inbox(waiting: &[Waiting]) -> String {
    inbox_with(waiting, &[])
}

/// The inbox, under the digest's `lines` and a button that subscribes this device to pushes;
/// see `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §3, §4.
#[must_use]
pub fn inbox_with(waiting: &[Waiting], digest: &[String]) -> String {
    let mut b = String::from("<h1>Waiting for you</h1>");
    if !digest.is_empty() {
        b.push_str("<h2>Digest</h2><ul>");
        for line in digest {
            let _ = write!(b, "<li>{}</li>", escape(line));
        }
        b.push_str("</ul>");
    }
    if waiting.is_empty() {
        b.push_str("<p>Nothing is waiting.</p>");
    } else {
        b.push_str("<ul>");
        for w in waiting.iter().rev() {
            let _ = write!(
                b,
                "<li><a href=\"{}\">{} — {} {}</a><br><small>{}</small></li>",
                escape(&w.path),
                escape(verb(&w.operation)),
                if w.operation.starts_with("mandate.") {
                    "mandate"
                } else {
                    "change"
                },
                w.number,
                escape(&w.title)
            );
        }
        b.push_str("</ul>");
    }
    b.push_str(
        "<p><button id=\"notify\" type=\"button\">Notify this device</button></p>\
         <script src=\"/approvals/push.js\"></script>",
    );
    shell("Waiting for you", &b)
}

/// The page offering `grant` on `change`, posting `token` and the owner's code back to
/// `/approve/<number>`.
#[must_use]
pub fn page(change: &Change, grant: &Grant, token: &str) -> String {
    let n = change.number;
    let mut b = String::new();
    let _ = write!(
        b,
        "<h1>{} — change {n}</h1><p><strong>{}</strong></p><dl>",
        escape(verb(&grant.operation)),
        escape(&change.title)
    );
    let mut row = |k: &str, v: String| {
        let _ = write!(b, "<dt>{}</dt><dd>{v}</dd>", escape(k));
    };
    row(
        "Operation",
        format!("<code>{}</code>", escape(&grant.operation)),
    );
    row(
        "Branch",
        format!("<code>{}</code>", escape(&change.branch())),
    );
    row(
        "Head",
        grant.head.as_deref().map_or_else(
            || "none pushed".to_string(),
            |h| format!("<code>{}</code>", escape(h)),
        ),
    );
    row("State", escape(change.state.name()));
    row("Proposed by", escape(&change.proposer));
    let jobs: Vec<String> = change
        .jobs
        .iter()
        .map(|j| {
            let status = serde_json::to_value(j.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            format!("{} — {}", escape(&j.name), escape(&status))
        })
        .collect();
    if !jobs.is_empty() {
        row("Checks", jobs.join("<br>"));
    }
    if !change.violations.is_empty() {
        let v: Vec<String> = change.violations.iter().map(|v| escape(v)).collect();
        row("Violations", v.join("<br>"));
    }
    row(
        "Scope",
        format!("<pre>{}</pre>", escape(&change.scope.join("\n"))),
    );
    row("Plan", format!("<pre>{}</pre>", escape(&change.plan)));
    if let Some(t) = &change.tiers {
        let tier = |x: &crate::approvals::Tiered| {
            let name = serde_json::to_value(x.tier)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            format!("{} ({})", escape(&name), escape(&x.rule))
        };
        row(
            "Tiers",
            format!(
                "authorize {}<br>merge {}<br>close {}",
                tier(&t.authorize),
                tier(&t.merge),
                tier(&t.close)
            ),
        );
    }
    let _ = write!(
        b,
        "</dl><form method=\"post\" action=\"/approve/{n}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">\
<input type=\"hidden\" name=\"operation\" value=\"{}\">\
{}<button type=\"submit\">{}</button></form>\
<p><small>This link works once, for this head only, until it expires. \
<a href=\"/approvals\">Everything waiting</a></small></p>",
        escape(token),
        escape(&grant.operation),
        if grant.needs_code() {
            CODE_FIELD
        } else {
            CONFIRM_NOTE
        },
        escape(verb(&grant.operation))
    );
    shell(&format!("{} — change {n}", verb(&grant.operation)), &b)
}

/// A page saying how it went.
#[must_use]
pub fn outcome(title: &str, message: &str) -> String {
    shell(
        title,
        &format!("<h1>{}</h1><pre>{}</pre>", escape(title), escape(message)),
    )
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes a form or query value; `+` is a space.
#[must_use]
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push(h * 16 + l);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Field `name` of an `application/x-www-form-urlencoded` body or a query string.
#[must_use]
pub fn field(form: &str, name: &str) -> Option<String> {
    form.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k) == name).then(|| decode(v))
    })
}

/// Whether a POST comes from this Worker's own pages: its `Origin` is `https://<host>`, or, when
/// the browser sends `Origin: null`, its `Sec-Fetch-Site` is `same-origin`.
#[must_use]
pub fn same_origin(origin: Option<&str>, fetch_site: Option<&str>, host: &str) -> bool {
    match origin {
        Some("null") | None => fetch_site == Some("same-origin"),
        Some(o) => o == format!("https://{host}"),
    }
}

/// The page granting `mandate`, posting `token` and the owner's code back to `/mandate/<id>`.
#[must_use]
pub fn mandate_page(m: &crate::mandates::Mandate, token: &str) -> String {
    let mut b = String::new();
    let _ = write!(
        b,
        "<h1>Grant mandate {}</h1><p><strong>{}</strong></p><dl>",
        m.id,
        escape(&m.summary)
    );
    let mut row = |k: &str, v: String| {
        let _ = write!(b, "<dt>{}</dt><dd>{v}</dd>", escape(k));
    };
    row("ADR", format!("<code>{}</code>", escape(&m.adr)));
    row(
        "Scope",
        format!("<pre>{}</pre>", escape(&m.scope.join("\n"))),
    );
    row(
        "May deploy",
        if m.may_deploy { "yes" } else { "no" }.to_string(),
    );
    row("Budget", format!("{} merges ({} used)", m.budget, m.used));
    row("Lasts", format!("{} hours from now", m.hours));
    let _ = write!(
        b,
        "</dl><p>Changes inside this scope are authorized, approved and merged without asking \
you, until the budget is used or it expires. The approval rules and policies always need your \
code.</p><form method=\"post\" action=\"/mandate/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{CODE_FIELD}\
<button type=\"submit\">Grant the mandate</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        m.id,
        escape(token)
    );
    shell(&format!("Grant mandate {}", m.id), &b)
}
