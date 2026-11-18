//! The approval family of presenters; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md.

use super::{
    card, code, count, detail, field, link, list, progress, row, scalars, section, sha, text, time,
};
use serde_json::Value;

/// The operation-name prefixes of this family; empty for the rest.
#[allow(dead_code)]
pub const PREFIXES: &[&str] = &[
    "approval.",
    "grant.",
    "mandate.",
    "lane.",
    "platform.",
    "pool.",
    "deploy.",
];

/// How many of this family's operations have no presenter yet; each brief that presents some lowers it.
#[allow(dead_code)]
pub const PENDING: usize = 0;

/// The operations this family presents.
pub const NAMES: &[&str] = &[
    "grant.list",
    "mandate.status",
    "platform.status",
    "pool.status",
    "platform.plan",
    "deploy.status",
    "approval.wait",
    "grant.request",
    "grant.rotate",
    "grant.revoke",
    "mandate.propose",
    "mandate.end",
    "lane.request",
    "lane.status",
    "lane.end",
    "platform.apply",
    "pool.pause",
    "pool.resume",
    "deploy.retry",
];

/// The view of operation `op`'s result, when this family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<Value> {
    if !NAMES.contains(&op) {
        return None;
    }
    let mut out = build(op, &r.result);
    if out["title"].as_str().is_none_or(|t| t.trim().is_empty()) {
        out["title"] = Value::String(op.to_string());
    }
    Some(out)
}

/// Whether `op` is presented as a list, a detail or (otherwise) a card.
fn shape(op: &str) -> &'static str {
    match op {
        "grant.list" | "lane.status" => "list",
        "mandate.status" | "platform.status" | "pool.status" | "platform.plan"
        | "deploy.status" => "detail",
        _ => "card",
    }
}

fn build(op: &str, v: &Value) -> Value {
    if op == "pool.pause" {
        return card("Pool paused", None, scalars(v), vec![]);
    }
    if !v.is_object() {
        return fallback(op, v);
    }
    match op {
        "grant.list" => grant_list(v),
        "mandate.status" => mandate_status(v),
        "lane.status" => lane_status(v),
        "lane.request" => card(
            &format!("Fast lane #{} asked", s(&v["lane"], "id")),
            opt_str(&v["lane"], "reason"),
            pending_link(&v["pending"]).into_iter().collect(),
            vec![],
        ),
        "lane.end" => card(
            &format!("Fast lane #{} {}", s(v, "id"), s(v, "state")),
            opt_str(v, "reason"),
            vec![
                field("waived", count(len(v, "waived"))),
                field("merged", count(len(v, "merged"))),
            ],
            vec![],
        ),
        "platform.status" => platform_status(v),
        "pool.status" => pool_status(v),
        "platform.plan" => platform_plan(v),
        "deploy.status" => deploy_status(v),
        "approval.wait" => card(
            "Waiting for the owner",
            opt_str(v, "message"),
            [
                vec![
                    field("operation", text(&s(&v["pending"], "operation"))),
                    field("change", text(&format!("!{}", s(&v["pending"], "number")))),
                ],
                pending_link(&v["pending"]).into_iter().collect(),
            ]
            .concat(),
            vec![],
        ),
        "grant.request" | "grant.rotate" | "grant.revoke" => card(
            "Asked the owner",
            opt_str(&v["pending"], "operation"),
            pending_link(&v["pending"]).into_iter().collect(),
            vec![],
        ),
        "mandate.propose" => card(
            &format!("Mandate #{} proposed", s(&v["mandate"], "id")),
            opt_str(&v["mandate"], "summary"),
            pending_link(&v["pending"]).into_iter().collect(),
            vec![],
        ),
        "mandate.end" => card(
            &format!("Mandate #{} {}", s(v, "id"), s(v, "state")),
            opt_str(v, "summary"),
            vec![field("used", count(num(v, "used")))],
            vec![],
        ),
        "platform.apply" => card(
            &format!("Apply #{} {}", s(&v["run"], "id"), s(&v["run"], "unit")),
            opt_str(&v["run"], "stage"),
            pending_link(&v["pending"]).into_iter().collect(),
            vec![],
        ),
        "pool.pause" => card("Pool paused", None, scalars(v), vec![]),
        "pool.resume" => card(
            "Resume offered",
            opt_str(&v["offer"], "pool"),
            [
                vec![field("expires", time(unum(&v["offer"], "expires")))],
                pending_link(&v["pending"]).into_iter().collect(),
            ]
            .concat(),
            vec![],
        ),
        _ => card(
            "Deploy retried",
            None,
            hex(&v["pipeline"], "target")
                .map(|t| field("target", sha(t)))
                .into_iter()
                .collect(),
            vec![],
        ),
    }
}

/// A result of the wrong shape, still as a valid view of its operation's kind.
fn fallback(op: &str, v: &Value) -> Value {
    match shape(op) {
        "list" => list(op, vec![], None),
        "detail" => detail(op, vec![section(None, scalars(v), None)], vec![]),
        _ => card(op, None, scalars(v), vec![]),
    }
}

/// `v[k]` as text: a string as is, a number written out, else empty.
fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `v[k]` when a string.
fn opt_str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v[k].as_str()
}

/// `v[k]` when a non-empty string.
fn set<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    opt_str(v, k).filter(|x| !x.is_empty())
}

/// `v[k]` as an unsigned number, 0 when absent.
fn unum(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}

/// `v[k]` as a count, 0 when absent.
fn num(v: &Value, k: &str) -> i64 {
    i64::try_from(unum(v, k)).unwrap_or(i64::MAX)
}

/// `v[k]` when a string of 7 to 64 lowercase hex digits.
fn hex<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    opt_str(v, k).filter(|x| {
        (7..=64).contains(&x.len())
            && x.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Micro-dollars as `$<whole>.<two digits>`, rounded down.
fn money(micros: u64) -> String {
    let cents = micros / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// The owner's approve link, when `p.link` is a string.
fn pending_link(p: &Value) -> Option<Value> {
    p["link"]
        .as_str()
        .map(|l| field("approve", link("Approve with passkey", l)))
}

fn grant_list(v: &Value) -> Value {
    let mut rows = Vec::new();
    if let Some(g) = v["grants"].as_object() {
        for (k, g) in g {
            rows.push(row(
                k,
                k,
                Some(&format!("by {}", s(g, "by"))),
                None,
                None,
                None,
            ));
        }
    }
    if let Some(l) = v["ledger"]["grants"].as_object() {
        for k in l.keys().filter(|k| v["grants"].get(k.as_str()).is_none()) {
            rows.push(row(k, k, None, Some("not granted"), None, None));
        }
    }
    list("Grants", rows, None)
}

fn len(v: &Value, k: &str) -> i64 {
    v[k].as_array()
        .map_or(0, |a| i64::try_from(a.len()).unwrap_or(i64::MAX))
}

fn lane_status(v: &Value) -> Value {
    let rows = v["lanes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| {
                    let id = s(l, "id");
                    let state = s(l, "state");
                    let sub = format!(
                        "{} · {} waived, {} merged",
                        s(l, "reason"),
                        len(l, "waived"),
                        len(l, "merged")
                    );
                    row(
                        &id,
                        &format!("Fast lane #{id}"),
                        Some(&sub),
                        Some(&state),
                        None,
                        None,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    list("Fast lanes", rows, None)
}

fn mandate_status(v: &Value) -> Value {
    let sections = v["mandates"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| {
                    let used = unum(m, "used");
                    let budget = unum(m, "budget");
                    let state = s(m, "state");
                    let usage = if budget > 0 {
                        let mark = match state.as_str() {
                            "paused" | "held" => "stalled",
                            "ended" => "done",
                            _ => "ok",
                        };
                        progress(used, budget, mark)
                    } else {
                        count(num(m, "used"))
                    };
                    let mut f = vec![
                        field("state", text(&state)),
                        field("used", usage),
                        field("hours", count(num(m, "hours"))),
                    ];
                    if let Some(e) = m["expires"].as_u64() {
                        f.push(field("expires", time(e)));
                    }
                    if let Some(w) = set(m, "why") {
                        f.push(field("why", text(w)));
                    }
                    section(
                        Some(&format!("#{} {}", s(m, "id"), s(m, "summary"))),
                        f,
                        None,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    detail("Mandates", sections, vec![])
}

fn platform_status(v: &Value) -> Value {
    let sections = v["runs"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| {
                    let mut f = vec![
                        field("stage", text(&s(r, "stage"))),
                        field("created", time(unum(r, "created"))),
                    ];
                    if let Some(h) = hex(r, "sha") {
                        f.push(field("sha", sha(h)));
                    }
                    if let Some(w) = set(r, "why") {
                        f.push(field("why", text(w)));
                    }
                    section(Some(&format!("#{} {}", s(r, "id"), s(r, "unit"))), f, None)
                })
                .collect()
        })
        .unwrap_or_default();
    detail("Platform runs", sections, vec![])
}

fn spent_of(m: &Value) -> String {
    format!("{} of {}", money(unum(m, "spent")), money(unum(m, "limit")))
}

fn pool_status(v: &Value) -> Value {
    let sections = v
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, p)| {
                    let state = p["state"]
                        .as_str()
                        .or_else(|| p["state"]["state"].as_str())
                        .unwrap_or("");
                    let mut f = vec![field("state", text(state))];
                    if p.get("ledger").is_some_and(|l| !l.is_null()) {
                        let l = &p["ledger"];
                        f.push(field("cycle", text(&spent_of(&l["cycle"]))));
                        f.push(field("month", text(&spent_of(&l["month"]))));
                        let open = match &l["tasks"] {
                            Value::Array(a) => {
                                a.iter().filter(|t| t.get("closed").is_none()).count()
                            }
                            Value::Object(o) => {
                                o.values().filter(|t| t.get("closed").is_none()).count()
                            }
                            _ => 0,
                        };
                        f.push(field(
                            "open tasks",
                            count(i64::try_from(open).unwrap_or(i64::MAX)),
                        ));
                        if let Some(p) = l["paused"].as_str() {
                            f.push(field("paused", text(p)));
                        }
                    }
                    section(Some(k), f, None)
                })
                .collect()
        })
        .unwrap_or_default();
    detail("Pools", sections, vec![])
}

fn platform_plan(v: &Value) -> Value {
    let run = &v["run"];
    let mut head = vec![
        field("stage", text(&s(run, "stage"))),
        field("created", time(unum(run, "created"))),
    ];
    head.extend(pending_link(&v["pending"]));
    let changes = run["changes"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, c)| field(&format!("change {}", i + 1), code(&c.to_string())))
                .collect()
        })
        .unwrap_or_default();
    detail(
        &format!("Plan #{} {}", s(run, "id"), s(run, "unit")),
        vec![
            section(Some("Run"), head, None),
            section(Some("Changes"), changes, None),
        ],
        vec![],
    )
}

fn deploy_status(v: &Value) -> Value {
    let p = &v["pipeline"];
    let mut sections = Vec::new();
    let pipe = ["deployed", "target"]
        .iter()
        .filter_map(|k| hex(p, k).map(|h| field(k, sha(h))))
        .collect();
    sections.push(section(Some("Pipeline"), pipe, None));
    let history = p["history"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(10)
                .map(|h| {
                    let mut t = s(h, "outcome");
                    let published: Vec<&str> = h["published"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    if !published.is_empty() {
                        t = format!("{t} · {}", published.join(", "));
                    }
                    let id: String = s(h, "sha").chars().take(12).collect();
                    field(&id, text(&t))
                })
                .collect()
        })
        .unwrap_or_default();
    sections.push(section(Some("History"), history, None));
    let mut mirror = Vec::new();
    if let Some(m) = hex(&v["mirror"], "mirrored") {
        mirror.push(field("mirrored", sha(m)));
    }
    mirror.push(field("failures", count(num(&v["mirror"], "failures"))));
    if let Some(b) = opt_str(v, "mirror_behind") {
        mirror.push(field("behind", text(b)));
    }
    sections.push(section(Some("Mirror"), mirror, None));
    if let Some(ps) = v["macos"]["providers"].as_array() {
        let f = ps
            .iter()
            .map(|x| {
                field(
                    &s(x, "provider"),
                    text(&format!("{}, {} min", s(x, "health"), unum(x, "minutes"))),
                )
            })
            .collect();
        sections.push(section(Some("macOS"), f, None));
    }
    detail("Deploys", sections, vec![])
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod tests;
