//! The change family of presenters; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md.

use super::{
    action, card, code, count, detail, field, list, needs, row, scalars, section, sha, text, time,
};
use serde_json::{Value, json};

/// The operation-name prefixes of this family; empty for the rest.
#[allow(dead_code)]
pub const PREFIXES: &[&str] = &["change.", "brief.", "session.", "task."];

/// How many of this family's operations have no presenter yet; each brief that presents some lowers it.
#[allow(dead_code)]
pub const PENDING: usize = 1;

/// The operations this family presents.
pub const NAMES: &[&str] = &[
    "change.list",
    "change.evictable",
    "brief.list",
    "task.list",
    "task.wait",
    "change.scope",
    "session.status",
    "brief.show",
    "change.propose",
    "change.pin",
    "change.front",
    "change.authorize",
    "change.pushed",
    "change.approve",
    "change.merge",
    "change.close",
    "change.yield",
    "change.note",
    "change.handoff",
    "brief.create",
    "brief.amend",
    "brief.review",
    "brief.escalate",
    "session.start",
    "session.end",
    "session.archived",
    "task.claim",
    "task.done",
];

/// A string key, empty when absent.
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// A string key, when it is one.
fn opt<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str()
}

/// A number key, 0 when absent.
fn u(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// A number key as a count.
fn n(v: &Value, key: &str) -> i64 {
    i64::try_from(u(v, key)).unwrap_or(0)
}

/// The array under a key, empty when absent.
fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map_or(&[], Vec::as_slice)
}

/// A text field for a string key, when it is one.
fn st(label: &str, v: &Value, key: &str) -> Option<Value> {
    opt(v, key).map(|t| field(label, text(t)))
}

/// A hash as a sha value when it is 7 to 64 lowercase hex digits, else as text.
fn hash(t: &str) -> Value {
    let hex = t.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    if (7..=64).contains(&t.len()) && hex {
        sha(t)
    } else {
        text(t)
    }
}

/// `title`, or `op` when it is empty or only spaces.
fn titled(op: &str, title: &str) -> String {
    if title.trim().is_empty() {
        op.to_string()
    } else {
        title.to_string()
    }
}

/// A change's `!<number>`, from a number key.
fn bang(v: &Value, key: &str) -> String {
    format!("!{}", u(v, key))
}

/// A card for a Change record.
fn change_card(op: &str, c: &Value) -> Value {
    let mut fields = Vec::new();
    if let Some(h) = opt(c, "head") {
        fields.push(field("head", hash(h)));
    }
    card(
        &titled(op, &format!("{} {}", bang(c, "number"), s(c, "title"))),
        Some(&format!("state: {}", s(c, "state"))),
        fields,
        vec![],
    )
}

fn status_action(number: u64) -> Value {
    action(
        "Open",
        "change.status",
        json!({ "number": number }),
        "plain",
        false,
    )
}

fn change_list(v: &Value) -> Value {
    let rows = items(v, "changes")
        .iter()
        .map(|c| {
            let number = u(c, "number");
            let stage = &c["stage"];
            let subtitle = opt(stage, "stage").or_else(|| opt(c, "state"));
            let nd = stage.is_object().then(|| needs(s(stage, "needs")));
            row(
                &number.to_string(),
                &format!("!{number} {}", s(c, "title")),
                subtitle,
                opt(c, "model"),
                nd,
                Some(status_action(number)),
            )
        })
        .collect();
    list("Changes", rows, None)
}

fn evictable(v: &Value) -> Value {
    let rows = items(v, "candidates")
        .iter()
        .map(|c| {
            let number = u(c, "number");
            let label = format!("!{number}");
            row(
                &label,
                &label,
                opt(&c["fields"], "state"),
                opt(&c["fields"], "kind"),
                None,
                Some(status_action(number)),
            )
        })
        .collect();
    list("Eviction candidates", rows, None)
}

fn brief_list(v: &Value) -> Value {
    let rows = items(v, "briefs")
        .iter()
        .map(|b| {
            let id = s(b, "id");
            row(
                id,
                &format!("{id} {}", s(b, "title")),
                Some(&format!("{} step {}", s(b, "adr"), s(b, "step"))),
                opt(b, "model"),
                None,
                Some(action(
                    "Open",
                    "brief.show",
                    json!({ "id": id }),
                    "plain",
                    false,
                )),
            )
        })
        .collect();
    list("Briefs", rows, None)
}

fn task_list(v: &Value) -> Value {
    let rows = items(v, "tasks")
        .iter()
        .map(|t| {
            let id = u(t, "id");
            let done = t["done"].as_bool().unwrap_or(false);
            let claimed = opt(t, "claimed_by").is_some();
            let badge = if done {
                Some("done")
            } else if claimed {
                Some("claimed")
            } else {
                None
            };
            let nd = (!done && !claimed).then_some("agent");
            row(
                &id.to_string(),
                &format!("#{id} {} on !{}", s(t, "kind"), u(t, "change")),
                opt(t, "note"),
                badge,
                nd,
                None,
            )
        })
        .collect();
    list("Tasks", rows, None)
}

fn scope(v: &Value) -> Value {
    let fields = items(v, "jobs")
        .iter()
        .map(|j| {
            let argv: Vec<&str> = items(j, "argv").iter().filter_map(Value::as_str).collect();
            field(s(j, "name"), code(&argv.join(" ")))
        })
        .collect();
    detail("Checks", vec![section(None, fields, None)], vec![])
}

fn session_status(op: &str, v: &Value) -> Value {
    let mut f: Vec<Value> = ["model", "role", "planner", "brief"]
        .iter()
        .filter_map(|k| st(k, v, k))
        .collect();
    if let Some(l) = v["events"]["listening"].as_bool() {
        f.push(field("listening", text(&l.to_string())));
    }
    let changes = items(v, "changes")
        .iter()
        .map(|c| {
            field(
                &format!("{} {}", bang(c, "number"), s(c, "title")),
                text(&format!(
                    "{} · needs {}",
                    s(&c["stage"], "stage"),
                    s(&c["stage"], "needs")
                )),
            )
        })
        .collect();
    let tasks = items(v, "tasks")
        .iter()
        .map(|t| {
            field(
                &format!("#{} {}", u(t, "id"), s(t, "kind")),
                text(s(t, "note")),
            )
        })
        .collect();
    detail(
        &titled(op, s(v, "session")),
        vec![
            section(Some("Session"), f, None),
            section(Some("Changes"), changes, None),
            section(Some("Tasks"), tasks, None),
        ],
        vec![],
    )
}

fn brief_show(op: &str, v: &Value) -> Value {
    let d = items(v, "revisions")
        .last()
        .map_or(&Value::Null, |r| &r["draft"]);
    let mut f: Vec<Value> = ["adr", "step", "model"]
        .iter()
        .filter_map(|k| st(k, d, k))
        .collect();
    if let Some(b) = opt(d, "base") {
        f.push(field("base", hash(b)));
    }
    f.push(field(
        "revisions",
        count(i64::try_from(items(v, "revisions").len()).unwrap_or(0)),
    ));
    let steps = items(d, "steps")
        .iter()
        .enumerate()
        .map(|(i, st_)| {
            field(
                &format!("{}. {}", i + 1, s(st_, "do")),
                text(s(st_, "done_when")),
            )
        })
        .collect();
    let decisions: Vec<&str> = items(d, "decisions")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let commit = &d["commit"];
    sections_brief(op, v, d, f, steps, &decisions.join("\n\n"), commit)
}

fn sections_brief(
    op: &str,
    v: &Value,
    d: &Value,
    f: Vec<Value>,
    steps: Vec<Value>,
    decisions: &str,
    commit: &Value,
) -> Value {
    let _ = d;
    let ct: Vec<Value> = st("title", commit, "title").into_iter().collect();
    detail(
        &titled(op, &format!("{} {}", s(v, "id"), s(d, "title"))),
        vec![
            section(Some("Brief"), f, None),
            section(Some("Steps"), steps, None),
            section(Some("Decisions"), vec![], Some(decisions)),
            section(Some("Commit"), ct, opt(commit, "plan")),
        ],
        vec![],
    )
}

fn revisions_card(v: &Value) -> Value {
    let revs = items(v, "revisions");
    let body = revs.last().and_then(|r| opt(&r["draft"], "title"));
    let mut f = Vec::new();
    if v["route"].is_object() {
        f.extend(st("route", &v["route"], "route"));
        f.extend(st("why", &v["route"], "why"));
    }
    card(&format!("{} r{}", s(v, "id"), revs.len()), body, f, vec![])
}

fn bool_text(label: &str, v: &Value, key: &str) -> Value {
    field(label, text(&v[key].as_bool().unwrap_or(false).to_string()))
}

fn session_start(v: &Value) -> Value {
    let mut f: Vec<Value> = ["model", "role"]
        .iter()
        .filter_map(|k| st(k, v, k))
        .collect();
    if let Some(t) = v["expires"].as_u64() {
        f.push(field("expires", time(t)));
    }
    f.extend(st("planner", v, "planner"));
    f.extend(st("brief", v, "brief"));
    card("Session started", opt(v, "label"), f, vec![])
}

fn session_end(v: &Value) -> Value {
    let mut f: Vec<Value> = st("ended", v, "ended").into_iter().collect();
    f.push(bool_text("clean", v, "clean"));
    f.extend(st("archiver", v, "archiver"));
    for h in items(v, "holds") {
        f.push(field(
            &format!("{} {}", bang(h, "number"), s(h, "title")),
            text(s(&h["remains"], "resume")),
        ));
    }
    card("Session ended", opt(v, "label"), f, vec![])
}

fn task_card(v: &Value) -> Value {
    let mut f: Vec<Value> = st("kind", v, "kind").into_iter().collect();
    if v["change"].is_number() {
        f.push(field("change", text(&bang(v, "change"))));
    }
    f.push(bool_text("done", v, "done"));
    if v["closed"].is_number() {
        f.push(field("closed", text(&bang(v, "closed"))));
    }
    card(&format!("Task #{}", u(v, "id")), opt(v, "note"), f, vec![])
}

fn pinned(v: &Value) -> Value {
    let t: Vec<String> = items(v, "pinned")
        .iter()
        .map(|p| format!("!{}", p.as_u64().unwrap_or(0)))
        .collect();
    card(
        "Pinned",
        None,
        vec![field("pinned", text(&t.join(", ")))],
        vec![],
    )
}

/// How an operation's result is shown when its shape isn't the expected one.
fn odd(op: &str, v: &Value) -> Value {
    let kind = if matches!(
        op,
        "change.list" | "change.evictable" | "brief.list" | "task.list" | "task.wait"
    ) {
        0
    } else if matches!(op, "change.scope" | "session.status" | "brief.show") {
        1
    } else {
        2
    };
    match kind {
        0 => list(op, vec![], None),
        1 => detail(op, vec![section(None, scalars(v), None)], vec![]),
        _ => card(op, None, scalars(v), vec![]),
    }
}

/// The view of operation `op`'s result, when this family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<serde_json::Value> {
    if !NAMES.contains(&op) {
        return None;
    }
    let v = &r.result;
    if !v.is_object() {
        return Some(odd(op, v));
    }
    let out = match op {
        "change.list" => change_list(v),
        "change.evictable" => evictable(v),
        "brief.list" => brief_list(v),
        "task.list" | "task.wait" => task_list(v),
        "change.scope" => scope(v),
        "session.status" => session_status(op, v),
        "brief.show" => brief_show(op, v),
        "change.propose" => {
            let mut f = vec![
                field("branch", code(s(v, "branch"))),
                field("state", text(s(&v["change"], "state"))),
            ];
            f.push(field(
                "overlaps",
                count(i64::try_from(items(v, "overlaps").len()).unwrap_or(0)),
            ));
            card(
                &titled(op, &format!("Proposed {}", bang(v, "number"))),
                opt(&v["change"], "title"),
                f,
                vec![],
            )
        }
        "change.pin" => pinned(v),
        "change.front" => card(
            &format!("Fronted {}", bang(v, "fronted")),
            opt(v, "message"),
            vec![],
            vec![],
        ),
        "change.authorize" | "change.pushed" | "change.close" | "change.approve" => {
            change_card(op, &v["change"])
        }
        "change.merge" => {
            if v["change"].is_object() {
                change_card(op, &v["change"])
            } else {
                card(
                    "Queued to merge",
                    opt(v, "message"),
                    vec![field("queued", count(n(v, "queued")))],
                    vec![],
                )
            }
        }
        "change.yield" => change_card(op, v),
        "change.note" => card(
            &format!("Noted on {}", bang(v, "noted")),
            None,
            vec![],
            vec![],
        ),
        "change.handoff" => card(
            &format!("Offered {}", bang(v, "offered")),
            None,
            vec![],
            vec![],
        ),
        "brief.create" | "brief.amend" => revisions_card(v),
        "brief.review" => card(
            &format!("Reviewed {}", bang(v, "change")),
            opt(v, "verdict"),
            st("brief", v, "brief").into_iter().collect(),
            vec![],
        ),
        "brief.escalate" => card(
            &format!("Escalated {}", bang(v, "change")),
            opt(v, "escalated"),
            st("brief", v, "brief").into_iter().collect(),
            vec![],
        ),
        "session.start" => session_start(v),
        "session.end" => session_end(v),
        "session.archived" => {
            let mut f: Vec<Value> = st("by", v, "by").into_iter().collect();
            if let Some(t) = v["at"].as_u64() {
                f.push(field("at", time(t)));
            }
            card(
                &titled(op, &format!("Session {}", s(v, "outcome"))),
                opt(v, "why"),
                f,
                vec![],
            )
        }
        "task.claim" | "task.done" => task_card(v),
        _ => return None,
    };
    Some(out)
}

#[cfg(test)]
#[path = "change_tests.rs"]
mod tests;
