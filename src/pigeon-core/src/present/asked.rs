//! What each owner page shows, as data for its view; see
//! .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md's 2026-11-18 amendment §2.

use serde_json::{Value, json};

use super::{code, field, sha, text};
use crate::approval::grants::Grant;
use crate::approval::lanes::{Lane, Line};
use crate::approval::mandates::Mandate;
use crate::change::Change;

/// One form field of a page.
fn input(key: &str, label: &str, kind: &str, required: bool, help: Option<&str>) -> Value {
    let mut o = json!({ "key": key, "label": label, "input": kind, "required": required });
    if let Some(h) = help {
        o["help"] = json!(h);
    }
    o
}

/// A page's content: its title, lead, fields, form fields, button, and whether it can refuse.
fn page(
    title: &str,
    body: &str,
    verb: &str,
    fields: &[Value],
    inputs: &[Value],
    refuse: bool,
    change: Option<u32>,
) -> Value {
    json!({
        "title": title,
        "body": body,
        "verb": verb,
        "fields": fields,
        "inputs": inputs,
        "refuse": refuse,
        "change": change,
    })
}

/// The name a serializable enum serializes to.
fn name_of<T: serde::Serialize>(t: &T) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The button's verb for `operation`.
fn verb(operation: &str) -> &str {
    match operation {
        "change.authorize" => "Approve the plan",
        "change.approve" => "Approve this head",
        "change.merge" => "Merge",
        "change.close" => "Close unmerged",
        "change.pin" => "Pin it against eviction",
        "change.front" => "Move it to the front of the merge queue",
        other => other,
    }
}

/// The page offering `grant` on `change`.
#[must_use]
pub fn change(change: &Change, grant: &Grant) -> Value {
    let n = change.number;
    let v = verb(&grant.operation);
    let mut fields = vec![
        field("Branch", code(&change.branch())),
        field("State", text(change.state.name())),
        field("Proposed by", text(&change.proposer)),
    ];
    if !change.jobs.is_empty() {
        let jobs: Vec<String> = change
            .jobs
            .iter()
            .map(|j| format!("{} — {}", j.name, name_of(&j.status)))
            .collect();
        fields.push(field("Checks", text(&jobs.join("; "))));
    }
    if !change.violations.is_empty() {
        fields.push(field("Violations", text(&change.violations.join("; "))));
    }
    fields.push(field("Scope", code(&change.scope.join("\n"))));
    fields.push(field("Plan", code(&change.plan)));
    if let Some(t) = &change.tiers {
        let tier =
            |x: &crate::approval::tiers::Tiered| format!("{} ({})", name_of(&x.tier), x.rule);
        fields.push(field(
            "Tiers",
            text(&format!(
                "authorize {}; merge {}; close {}",
                tier(&t.authorize),
                tier(&t.merge),
                tier(&t.close)
            )),
        ));
    }
    let mut inputs = Vec::new();
    if grant.operation == "change.authorize" {
        for a in &change.asks {
            let help = (!a.help.is_empty()).then_some(a.help.as_str());
            inputs.push(input(
                &format!("ask_{}", a.key),
                &a.label,
                "text",
                true,
                help,
            ));
        }
    }
    if grant.needs_code() && grant.operation != "change.close" {
        inputs.push(input(
            "review",
            "Keep the merge for me to approve",
            "toggle",
            false,
            None,
        ));
    }
    page(
        &format!("{v} — change {n}"),
        &change.title,
        v,
        &fields,
        &inputs,
        false,
        Some(n),
    )
}

/// The page granting `m`.
#[must_use]
pub fn mandate(m: &Mandate) -> Value {
    page(
        &format!("Grant mandate {}", m.id),
        &m.summary,
        "Grant the mandate",
        &[
            field("ADR", code(&m.adr)),
            field("Scope", code(&m.scope.join("\n"))),
            field("May deploy", text(if m.may_deploy { "yes" } else { "no" })),
            field(
                "Budget",
                text(&format!("{} merges ({} used)", m.budget, m.used)),
            ),
            field("Lasts", text(&format!("{} hours from now", m.hours))),
        ],
        &[],
        false,
        None,
    )
}

/// The page opening fast lane `l`.
#[must_use]
pub fn lane(l: &Lane) -> Value {
    let line = match &l.line {
        Line::Session(s) => text(&format!("session {s}")),
        Line::Changes(n) => {
            let n: Vec<String> = n.iter().map(|n| format!("!{n}")).collect();
            text(&format!("changes {}", n.join(", ")))
        }
        Line::Paths(g) => code(&g.join("\n")),
    };
    let mut fields = vec![
        field("Line of work", line),
        field("Lasts", text(&format!("{} minutes from now", l.minutes))),
    ];
    if let Some(open) = l.extends {
        fields.push(field("Widens", text(&format!("open lane {open}"))));
    }
    fields.push(field("Asked by", text(&l.asked_by)));
    page(
        &format!("Open fast lane {}", l.id),
        &l.reason,
        "Open the fast lane",
        &fields,
        &[],
        false,
        None,
    )
}

/// The page running grant request `o` on the ledger's `entry`.
#[must_use]
pub fn grant(o: &crate::minting::Offer, entry: Option<&crate::ledger::Grant>) -> Value {
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
    page(
        &title,
        what,
        "Approve",
        &[field("Spec", code(&spec))],
        &[],
        false,
        None,
    )
}

/// The page granting CI window offer `o`.
#[must_use]
pub fn ci_window(o: &crate::ci_window::Offer) -> Value {
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
    page(&title, &what, "Approve", &[], &[], false, None)
}

/// What `/service-bind/<id>` shows: the Administrator binds, or revokes, a service token by its
/// client ID's hash.
#[must_use]
pub fn service_bind(o: &crate::member::bind::Offer) -> Value {
    page(
        &o.title(),
        if o.unbind {
            "The service token stops naming any service account at once: its calls are no one's, and refused once the member check decides."
        } else {
            "Calls with this service token act as this service account, held to its scopes, tools and denials. Only the Administrator binds a token, and only its client ID's hash is kept."
        },
        "Approve",
        &[],
        &[],
        false,
        None,
    )
}

/// The page resuming capacity pool offer `o`.
#[must_use]
pub fn pool_resume(o: &crate::pool::resume::Offer) -> Value {
    page(
        &format!("Resume the {} pool", o.pool),
        "The pool admits new tasks again, within its caps. Check why it paused before resuming it: pool_status shows the reason.",
        "Approve",
        &[],
        &[],
        false,
        None,
    )
}

/// The page granting the next step of platform run `run`; `needs` is the unit's permissions or
/// why there are none, `mint` whether one proof mints the account token.
#[must_use]
pub fn platform(
    run: &crate::platform::platforms::Run,
    needs: &Result<Vec<crate::minting::Wanted>, String>,
    mint: bool,
) -> Value {
    use crate::platform::platforms::Stage;
    let planning = run.stage == Stage::Offered;
    let minting = planning && mint && needs.is_ok();
    let title = if planning {
        format!("Plan {}", run.unit)
    } else {
        format!("Apply the plan of {}", run.unit)
    };
    let mut fields = Vec::new();
    if planning {
        match needs {
            Ok(wanted) => {
                for w in wanted {
                    fields.push(field(
                        &w.name,
                        text(&format!("{} ({})", w.permissions.join(", "), w.scope)),
                    ));
                }
            }
            Err(why) => fields.push(field("Permissions", text(&format!("none listed: {why}")))),
        }
    } else {
        let changes: Vec<String> = run
            .changes
            .iter()
            .map(|c| {
                format!(
                    "{} {}",
                    c["action"].as_str().unwrap_or_default(),
                    c["address"].as_str().unwrap_or_default()
                )
            })
            .collect();
        fields.push(field("Plan", code(run.plan.as_deref().unwrap_or_default())));
        fields.push(field("Commit", sha(run.sha.as_deref().unwrap_or_default())));
        fields.push(field("Changes", code(&changes.join("\n"))));
    }
    let verb = if minting {
        "Mint and plan"
    } else if planning {
        "Plan it"
    } else {
        "Apply"
    };
    let inputs = if minting {
        vec![input(
            "account_token",
            "Account token",
            "secret",
            false,
            Some("Break-glass: paste a token instead of minting one"),
        )]
    } else if planning {
        vec![input(
            "account_token",
            "Account token",
            "secret",
            true,
            None,
        )]
    } else {
        Vec::new()
    };
    page(
        &title,
        &format!("Platform run {}.", run.id),
        verb,
        &fields,
        &inputs,
        !planning,
        None,
    )
}

#[cfg(test)]
#[path = "asked_tests.rs"]
mod tests;
