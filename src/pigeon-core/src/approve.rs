//! The approval page and its inbox: server-rendered HTML with no scripts, every value escaped;
//! see `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::approval::grants::Grant;
use crate::change::Change;

/// The page's Content-Security-Policy: no scripts, no external loads, forms only to itself;
/// `img-src data:` for the inline avatar, per `.claude/decisions/2026-11-16-runtime-owner-pages-app-look.md` §3.
pub const CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// A page that takes a proof: [`CSP`], plus `/passkey.js` and its fetch of a challenge; see
/// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §4.
pub const PROOF_CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The inbox's policy: [`CSP`], plus its own script, its fetches and its service worker.
pub const INBOX_CSP: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; worker-src 'self'; manifest-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The inbox's web app manifest, so iOS installs it to the Home Screen, where Web Push works.
pub const MANIFEST: &str = r##"{"name":"pigeon approvals","short_name":"pigeon","start_url":"/app","scope":"/","display":"standalone","background_color":"#14121A","theme_color":"#14121A"}"##;

/// The inbox's `<head>` additions: its manifest and iOS's standalone flags.
const INBOX_HEAD: &str = "<link rel=\"manifest\" href=\"/approvals/manifest.json\">\
<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">\
<meta name=\"apple-mobile-web-app-title\" content=\"pigeon\">";

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
/// What a proof on a change's page also covers, and the box that keeps the merge for the owner;
/// see `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §3.
const COVERS: &str = "<p><small>This proof also covers the change's later stages, at no more than \
the code tier, for 24 hours. A head that touches a secret, or destroys anything, still asks.</small></p>\
<p><label><input type=\"checkbox\" name=\"review\" value=\"1\"> Keep the merge for me to approve</label></p>";

/// What approving a head also does; see the approvals ADR's 2026-11-12 amendment.
const MERGES_NOTE: &str = "<p><small>Approving this head also queues its merge, unless you keep \
the merge. A head that changes before it merges asks again.</small></p>";

/// Above a change's asks; see `.claude/decisions/2026-11-13-security-approvals-ask-owner-inputs.md` §4.
const ASKS_NOTE: &str = "<p><strong>This change asks for values.</strong> Each answer is stored on \
the change in plain text and will be committed to the public repository. Never enter a secret.</p>";

/// The fields for `change`'s asks, when `grant` authorizes its plan; empty otherwise.
#[must_use]
pub fn asks_fields(change: &Change, grant: &Grant) -> String {
    if grant.operation != "change.authorize" || change.asks.is_empty() {
        return String::new();
    }
    let mut b = String::from(ASKS_NOTE);
    for a in &change.asks {
        let _ = write!(
            b,
            "<p><label>{}<br><input name=\"ask_{}\" required autocomplete=\"off\" \
spellcheck=\"false\" minlength=\"{}\" maxlength=\"{}\" pattern=\"{}\"></label>{}</p>",
            escape(&a.label),
            escape(&a.key),
            a.min,
            a.max,
            escape(&a.pattern()),
            if a.help.is_empty() {
                String::new()
            } else {
                format!("<br><small>{}</small>", escape(&a.help))
            }
        );
    }
    b
}

/// The answers a form posts for `change`'s asks, by key.
#[must_use]
pub fn answers_from(change: &Change, form: &str) -> std::collections::BTreeMap<String, String> {
    change
        .asks
        .iter()
        .filter_map(|a| {
            Some((
                a.key.clone(),
                field(form, &format!("ask_{}", a.key))?.trim().to_string(),
            ))
        })
        .collect()
}

const CONFIRM_NOTE: &str =
    "<p>This is a low-risk operation, so one tap approves it; no code is needed.</p>";

/// A verb for the button.
fn verb(operation: &str) -> &'static str {
    match operation {
        "change.authorize" => "Approve the plan",
        "change.approve" => "Approve this head",
        "change.merge" => "Merge",
        "change.close" => "Close unmerged",
        "change.pin" => "Pin it against eviction",
        "change.front" => "Move it to the front of the merge queue",
        crate::control::BREAK_GLASS => "Run its code checks on Cloudflare",
        "mandate.grant" => "Grant the mandate",
        "lane.grant" => "Open the fast lane",
        crate::ci_window::OPERATION => "Move code checks to Cloudflare",
        "grant.request" => "Make the grant",
        "grant.rotate" => "Rotate the grant",
        "grant.revoke" => "Revoke the grant",
        _ => "Confirm",
    }
}

fn shell(title: &str, body: &str) -> String {
    shell_with(title, "", body)
}

/// A page with `head` added to its `<head>`, in the app's look; see [`crate::look`].
fn shell_with(title: &str, head: &str, body: &str) -> String {
    crate::look::page(title, head, body)
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
                } else if w.operation.starts_with("lane.") {
                    "fast lane"
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
         <p><small>On an iPhone or iPad: Share, then Add to Home Screen, and tap this from the \
         icon.</small></p><script src=\"/approvals/push.js\"></script>",
    );
    shell_with("Waiting for you", INBOX_HEAD, &b)
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
        let tier = |x: &crate::approval::tiers::Tiered| {
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
{}{}<button type=\"submit\">{}</button></form>\
<p><small>This link works once, for this head only, until it expires. \
<a href=\"/approvals\">Everything waiting</a></small></p>",
        escape(token),
        escape(&grant.operation),
        asks_fields(change, grant),
        proof_fields(grant, token),
        escape(verb(&grant.operation))
    );
    shell(&format!("{} — change {n}", verb(&grant.operation)), &b)
}

/// The page's proof: a passkey or the code at the code tier, a note at the confirm tier.
fn proof_fields(grant: &Grant, token: &str) -> String {
    if grant.needs_code() && grant.operation != "change.close" {
        format!(
            "{}{CODE_FIELD}{}{COVERS}",
            passkey_button(&crate::approval::passkeys::grant_purpose(token)),
            if grant.operation == "change.approve" {
                MERGES_NOTE
            } else {
                ""
            }
        )
    } else if grant.needs_code() {
        format!(
            "{}{CODE_FIELD}",
            passkey_button(&crate::approval::passkeys::grant_purpose(token))
        )
    } else {
        CONFIRM_NOTE.to_string()
    }
}

/// The button that proves the owner with a passkey instead of the code, for `purpose`; hidden
/// until `/passkey.js` finds `WebAuthn`.
fn passkey_button(purpose: &str) -> String {
    format!("{}{PASSKEY_SCRIPT}", proof_button(purpose, "Use a passkey"))
}

/// The proof pages' one script.
const PASSKEY_SCRIPT: &str = "<script src=\"/passkey.js\" defer></script>";

/// A passkey button labelled `label` for `purpose`, without the script, for a page with several.
fn proof_button(purpose: &str, label: &str) -> String {
    format!(
        "<p><button type=\"button\" data-passkey=\"prove\" data-purpose=\"{}\" hidden>{}</button> \
<small data-passkey-status></small></p>",
        escape(purpose),
        escape(label)
    )
}

/// Where the passkey-only cutover stands: what it still lacks, and since when codes are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cutover<'a> {
    /// What is still needed before codes are refused, one line each.
    pub missing: &'a [String],
    /// When codes were first refused.
    pub since: Option<u64>,
}

/// A passkey as `/passkeys` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Its credential ID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// When it was registered.
    pub created: u64,
    /// When it last proved the owner.
    pub used: Option<u64>,
    /// Whether it is synced; unknown for older ones.
    pub synced: Option<bool>,
}

/// `/passkeys`: the owner's passkeys, each removed with a proof, and a form to add one, proven by
/// an existing passkey (`adding`, once it has been) or the code; and `said`. See
/// `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.
#[must_use]
pub fn passkeys_page(
    keys: &[Listed],
    said: Option<&str>,
    adding: bool,
    cutover: &Cutover,
) -> String {
    use std::fmt::Write as _;
    let mut b = String::from("<h1>Passkeys</h1>");
    if let Some(said) = said {
        let _ = write!(b, "<p><strong>{}</strong></p>", escape(said));
    }
    b.push_str(
        "<p>A passkey approves on these pages. Adding one takes an existing passkey or the \
authenticator code; removing one takes either.</p>",
    );
    if keys.is_empty() {
        b.push_str("<p>None yet.</p>");
    } else {
        b.push_str("<ul>");
        for k in keys {
            let kind = match k.synced {
                Some(true) => ", synced",
                Some(false) => ", this device only",
                None => "",
            };
            let _ = write!(
                b,
                "<li>{} <small>(registered {}{}{kind})</small>\
<form method=\"post\" action=\"/passkeys\"><input type=\"hidden\" name=\"remove\" value=\"{}\">\
{}{CODE_FIELD}<button type=\"submit\">Remove</button></form></li>",
                escape(&k.name),
                k.created,
                k.used.map_or(String::new(), |u| format!(", last used {u}")),
                escape(&k.id),
                proof_button(
                    &crate::approval::passkeys::remove_purpose(&k.id),
                    "Remove with a passkey"
                )
            );
        }
        b.push_str("</ul>");
    }
    b.push_str("<h2>Passkey-only</h2>");
    match (cutover.since, cutover.missing.is_empty()) {
        (Some(at), _) => {
            let _ = write!(
                b,
                "<p>Codes are refused since {at}; a passkey is the only proof.</p>"
            );
        }
        (None, true) => b.push_str(
            "<p>Ready: everything the cutover needs is on record. Codes still work until it is \
made.</p>",
        ),
        (None, false) => {
            b.push_str(
                "<p>Codes still work. Before they can be refused, this is still needed:</p><ul>",
            );
            for line in cutover.missing {
                let _ = write!(b, "<li>{}</li>", escape(line));
            }
            b.push_str("</ul>");
        }
    }
    b.push_str("<h2>Add a passkey</h2>");
    if !keys.is_empty() && !adding {
        let _ = write!(
            b,
            "<form method=\"post\" action=\"/passkeys\"><input type=\"hidden\" name=\"add\" value=\"1\">{}</form>",
            proof_button(
                crate::approval::passkeys::ADD,
                "Prove with an existing passkey first"
            )
        );
    }
    let _ = write!(
        b,
        "<form method=\"post\" action=\"/passkeys\">\
<p><label>Name<br><input name=\"name\" maxlength=\"64\" required></label></p>{}\
<p><button type=\"button\" data-passkey=\"register\" data-purpose=\"{}\" hidden>Create a passkey</button> \
<small data-passkey-status></small></p></form>{PASSKEY_SCRIPT}\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        if adding { "" } else { CODE_FIELD },
        escape(crate::approval::passkeys::REGISTER)
    );
    shell("Passkeys", &b)
}

/// `/passkeys/enroll`: registers one passkey with a one-time enrolment token, and `said`; see
/// `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §3.
#[must_use]
pub fn enroll_page(said: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut b = String::from("<h1>Enrol a passkey</h1>");
    if let Some(said) = said {
        let _ = write!(b, "<p><strong>{}</strong></p>", escape(said));
    }
    let _ = write!(
        b,
        "<p>For when no passkey is left. <code>pigeon keys enroll</code> prints a \
token that works once, for at most an hour, after its other line is put as the \
<code>PASSKEY_ENROLL</code> secret.</p>\
<form method=\"post\" action=\"/passkeys/enroll\">\
<p><label>Enrolment token<br><input name=\"token\" type=\"password\" autocomplete=\"off\" \
minlength=\"32\" required></label></p>\
<p><label>Name<br><input name=\"name\" maxlength=\"64\" required></label></p>\
<p><label><input type=\"checkbox\" name=\"replace\" value=\"1\"> Remove every existing passkey</label></p>\
<p><button type=\"button\" data-passkey=\"register\" data-purpose=\"{}\" hidden>Create a passkey</button> \
<small data-passkey-status></small></p></form>{PASSKEY_SCRIPT}\
<p><small><a href=\"/passkeys\">Passkeys</a></small></p>",
        escape(crate::approval::passkeys::REGISTER)
    );
    shell("Enrol a passkey", &b)
}

/// `/app/link`: links the device named `name`, whose key has `fingerprint`, once the owner
/// proves it; see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §2.
#[must_use]
pub fn link_page(name: &str, key: &str, fingerprint: &str) -> String {
    let purpose = format!("device:{fingerprint}");
    let b = format!(
        "<h1>Link this iPhone</h1><dl><dt>Device</dt><dd>{}</dd><dt>Key</dt><dd><code>{}</code></dd></dl>\
<p>Linking lets the Pigeon app read this instance and ask for your passkey to approve. \
Check the key matches the one the app shows.</p>\
<form method=\"post\" action=\"/app/link\"><input type=\"hidden\" name=\"name\" value=\"{}\">\
<input type=\"hidden\" name=\"key\" value=\"{}\">{}{CODE_FIELD}<button type=\"submit\">Link</button></form>\
<p><small><a href=\"/devices\">Linked devices</a></small></p>",
        escape(name),
        escape(fingerprint),
        escape(name),
        escape(key),
        passkey_button(&purpose)
    );
    shell("Link this iPhone", &b)
}

/// `/devices`: the linked devices, each removable with a proof, and `said`.
#[must_use]
pub fn devices_page(devices: &[(String, String, u64, u64)], said: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut b = String::from("<h1>Linked devices</h1>");
    if let Some(said) = said {
        let _ = write!(b, "<p><strong>{}</strong></p>", escape(said));
    }
    if devices.is_empty() {
        b.push_str("<p>None.</p>");
    } else {
        b.push_str("<ul>");
        for (id, name, linked, seen) in devices {
            let _ = write!(
                b,
                "<li>{} <small>(<code>{}</code>, linked {linked}, last seen {seen})</small>\
<form method=\"post\" action=\"/devices\"><input type=\"hidden\" name=\"remove\" value=\"{}\">\
{}{CODE_FIELD}<button type=\"submit\">Unlink</button></form></li>",
                escape(name),
                escape(id),
                escape(id),
                proof_button(&format!("device:{id}"), "Unlink with a passkey")
            );
        }
        b.push_str("</ul>");
        b.push_str(PASSKEY_SCRIPT);
    }
    shell("Linked devices", &b)
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
pub fn mandate_page(m: &crate::approval::mandates::Mandate, token: &str) -> String {
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
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}\
<button type=\"submit\">Grant the mandate</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        m.id,
        escape(token),
        passkey_button(&crate::approval::passkeys::grant_purpose(token))
    );
    shell(&format!("Grant mandate {}", m.id), &b)
}

/// The page running grant request `o` on the ledger's `entry`, posting `token` back to
/// `/grants/<id>` with the code; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.
#[must_use]
pub fn grant_page(
    o: &crate::minting::Offer,
    entry: Option<&crate::ledger::Grant>,
    token: &str,
) -> String {
    use crate::minting::Kind;
    let (title, what) = match o.kind {
        Kind::Request => (
            format!("Make grant `{}`", o.name),
            "The control plane mints or generates it, puts it at its route, verifies it and records it. Its value is never shown or kept.",
        ),
        Kind::Rotate => (
            format!("Rotate grant `{}`", o.name),
            "A new value is made, put and verified; then the old token is revoked. If any step fails, the old one stays.",
        ),
        Kind::Revoke => (
            format!("Revoke grant `{}`", o.name),
            "Its token is revoked and its value removed from its route.",
        ),
    };
    let spec = entry
        .and_then(|e| serde_json::to_string_pretty(e).ok())
        .unwrap_or_else(|| "(not in the deployed ledger)".into());
    let b = format!(
        "<h1>{}</h1><p>{}</p><pre>{}</pre><form method=\"post\" action=\"/grants/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}\
<button type=\"submit\">Approve</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        escape(&title),
        escape(what),
        escape(&spec),
        o.id,
        escape(token),
        passkey_button(&crate::approval::passkeys::grant_purpose(token))
    );
    shell(&title, &b)
}

/// The page granting CI window offer `o`, posting `token` back to `/ci-window/<id>` with the
/// code; see `.claude/decisions/2026-11-10-development-code-checks-github-actions.md`, its
/// 2026-11-11 amendment.
#[must_use]
pub fn ci_window_page(o: &crate::ci_window::Offer, token: &str) -> String {
    let (title, what) = if o.hours == 0 {
        (
            "Close the CI window".to_string(),
            "Code checks go back to GitHub Actions for every new head.".to_string(),
        )
    } else {
        (
            format!("Code checks on Cloudflare for {} hours", o.hours),
            format!(
                "For {} hours from now, every change's code checks, and the merge gate's, run on the Checker instead of GitHub Actions. Changes with checks queued or running on GitHub move now. It closes by itself.",
                o.hours
            ),
        )
    };
    let b = format!(
        "<h1>{}</h1><p>{}</p><form method=\"post\" action=\"/ci-window/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}\
<button type=\"submit\">Approve</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        escape(&title),
        escape(&what),
        o.id,
        escape(token),
        passkey_button(&crate::approval::passkeys::grant_purpose(token))
    );
    shell(&title, &b)
}

/// A platform plan's page section: the unit's permissions from `needs`, or why there are none,
/// and how the token is made and ended, `minting` or pasted.
fn plan_needs(b: &mut String, needs: &Result<Vec<crate::minting::Wanted>, String>, minting: bool) {
    match needs {
        Ok(wanted) => {
            b.push_str(if minting {
                "<p>One passkey proof mints this run's account token through \
<code>granter</code>, with only these permissions:</p><ul>"
            } else {
                "<p>The engine plans this stack with an account token you make in the \
dashboard, with only these permissions:</p><ul>"
            });
            for w in wanted {
                let _ = write!(
                    b,
                    "<li><code>{}</code>: {} ({})</li>",
                    escape(&w.name),
                    escape(&w.permissions.join(", ")),
                    escape(&w.scope)
                );
            }
            b.push_str("</ul>");
        }
        Err(why) => {
            let _ = write!(
                b,
                "<p>The deployed grants ledger lists no permissions for this unit: {}. \
Make its account token in the dashboard with what its stack needs.</p>",
                escape(why)
            );
        }
    }
    b.push_str(if minting {
        "<p>It is verified and sealed for this run alone, revoked when the run is applied, \
fails, is refused or expires, and expires in 24 hours whatever happens. It plans \
<code>main</code>'s head when the job starts; the apply's page names that commit.</p>"
    } else {
        "<p>The token is sealed for this run alone and erased after the apply, a refusal \
or 24 hours; revoke it in the dashboard afterwards. It plans <code>main</code>'s head when the \
job starts; the apply's page names that commit.</p>"
    });
}

/// The page resuming capacity pool offer `o`, posting `token` back to
/// `/pool-resume/<pool>/<id>` with the passkey; see
/// `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md` §8.
#[must_use]
pub fn pool_resume_page(o: &crate::pool::resume::Offer, token: &str) -> String {
    let title = format!("Resume the {} pool", o.pool);
    let what = "The pool admits new tasks again, within its caps. Check why it paused before resuming it: pool_status shows the reason.";
    let b = format!(
        "<h1>{}</h1><p>{}</p><form method=\"post\" action=\"{}{}/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}\
<button type=\"submit\">Approve</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        escape(&title),
        escape(what),
        crate::pool::resume::PAGE,
        escape(&o.pool),
        o.id,
        escape(token),
        passkey_button(&crate::approval::passkeys::grant_purpose(token))
    );
    shell(&title, &b)
}

/// The page granting the next step of platform run `run`, posting `token` back to
/// `/platform/<id>`: for its plan, the owner's code and one-time account token; for its apply,
/// the plan's changes, the code, and a refusal. A plan's page lists `needs`, the unit's capabilities
/// as `main`'s deploy reported them, or why there are none; see
/// `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment, and
/// `.claude/decisions/2026-11-17-security-platform-run-minted-token.md`, its 2026-11-17 amendment.
/// With `mint` and a set, one passkey proof, bound to that set, mints the token, and pasting one
/// is break-glass (§4, §6).
#[must_use]
pub fn platform_page(
    run: &crate::platform::platforms::Run,
    token: &str,
    needs: &Result<Vec<crate::minting::Wanted>, String>,
    mint: bool,
) -> String {
    use crate::platform::platforms::Stage;
    let mut b = String::new();
    let planning = run.stage == Stage::Offered;
    let minting = planning && mint && needs.is_ok();
    let purpose = match needs {
        Ok(wanted) if minting => {
            let names: Vec<String> = wanted.iter().map(|w| w.name.clone()).collect();
            crate::approval::passkeys::mint_purpose(token, &names)
        }
        _ => crate::approval::passkeys::grant_purpose(token),
    };
    let title = if planning {
        format!("Plan {}", run.unit)
    } else {
        format!("Apply the plan of {}", run.unit)
    };
    let _ = write!(
        b,
        "<h1>{}</h1><p>Platform run {}.</p>",
        escape(&title),
        run.id
    );
    if planning {
        plan_needs(&mut b, needs, minting);
    } else {
        let _ = write!(
            b,
            "<p>Plan <code>{}</code> of commit <code>{}</code>, which the apply checks out, {} \
change(s). Existing resources are imported; nothing is destroyed (a plan that destroys is \
refused before it reaches you).</p><pre>",
            escape(run.plan.as_deref().unwrap_or_default()),
            escape(run.sha.as_deref().unwrap_or_default()),
            run.changes.len()
        );
        for c in &run.changes {
            let _ = writeln!(
                b,
                "{} {}",
                escape(c["action"].as_str().unwrap_or_default()),
                escape(c["address"].as_str().unwrap_or_default())
            );
        }
        b.push_str("</pre>");
    }
    let _ = write!(
        b,
        "<form method=\"post\" action=\"/platform/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}",
        run.id,
        escape(token),
        passkey_button(&purpose)
    );
    if minting {
        b.push_str(
            "<details><summary>Break-glass: paste a token instead</summary><p><label>Account \
token<br><input name=\"account_token\" type=\"password\" autocomplete=\"off\" minlength=\"20\">\
</label></p></details>\
<button type=\"submit\" name=\"decision\" value=\"grant\">Mint and plan</button></form>",
        );
    } else if planning {
        b.push_str(
            "<p><label>Account token<br><input name=\"account_token\" type=\"password\" \
autocomplete=\"off\" minlength=\"20\" required></label></p>\
<button type=\"submit\" name=\"decision\" value=\"grant\">Plan it</button></form>",
        );
    } else {
        b.push_str(
            "<button type=\"submit\" name=\"decision\" value=\"grant\">Apply</button> \
<button type=\"submit\" name=\"decision\" value=\"refuse\" formnovalidate>Refuse</button></form>",
        );
    }
    b.push_str("<p><small><a href=\"/approvals\">Everything waiting</a></small></p>");
    shell(&title, &b)
}

/// The page granting fast-lane request `l`, posting `token` back to `/lane/<id>` with the
/// owner's passkey; see `.claude/decisions/2026-11-17-security-owner-fast-lane-window.md` §1.
#[must_use]
pub fn lane_page(l: &crate::approval::lanes::Lane, token: &str) -> String {
    use crate::approval::lanes::Line;
    let mut b = String::new();
    let _ = write!(
        b,
        "<h1>Open fast lane {}</h1><p><strong>{}</strong></p><dl>",
        l.id,
        escape(&l.reason)
    );
    let mut row = |k: &str, v: String| {
        let _ = write!(b, "<dt>{}</dt><dd>{v}</dd>", escape(k));
    };
    let line = match &l.line {
        Line::Session(s) => format!("session <code>{}</code>", escape(s)),
        Line::Changes(n) => {
            let n: Vec<String> = n.iter().map(|n| format!("!{n}")).collect();
            format!("changes {}", escape(&n.join(", ")))
        }
        Line::Paths(g) => format!("<pre>{}</pre>", escape(&g.join("\n"))),
    };
    row("Line of work", line);
    row("Lasts", format!("{} minutes from now", l.minutes));
    if let Some(open) = l.extends {
        row("Widens", format!("open lane {open}"));
    }
    row("Asked by", escape(&l.asked_by));
    let _ = write!(
        b,
        "</dl><p>Its changes' code checks (fmt, clippy, tests, builds) are waived and run after \
they merge, and its changes go to the front of the merge queue. The bounds, the policies, the \
approval rules, secrets and guarded paths are never waived, and each change still needs its \
approval.</p><form method=\"post\" action=\"/lane/{}\">\
<input type=\"hidden\" name=\"grant\" value=\"{}\">{}{CODE_FIELD}\
<button type=\"submit\">Open the fast lane</button></form>\
<p><small><a href=\"/approvals\">Everything waiting</a></small></p>",
        l.id,
        escape(token),
        passkey_button(&crate::approval::passkeys::grant_purpose(token))
    );
    shell(&format!("Open fast lane {}", l.id), &b)
}
