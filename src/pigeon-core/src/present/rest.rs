//! The rest family of presenters; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md.

use super::{
    action, card, code, count, detail, duration, field, link, list, needs, row, scalars, section,
    sha, text, time,
};
use serde_json::Value;

/// The operation-name prefixes of this family; empty for the rest.
#[allow(dead_code)]
pub const PREFIXES: &[&str] = &[];

/// How many of this family's operations have no presenter yet; each brief that presents some lowers it.
#[allow(dead_code)]
pub const PENDING: usize = 0;

/// The operations this family presents.
pub const NAMES: &[&str] = &[
    "git.tokens",
    "bench.list",
    "events.wait",
    "provider.tools",
    "feed.read",
    "instance.show",
    "architecture.describe",
    "policy.check",
    "infra.validate",
    "ios.status",
    "apple.check",
    "digest.show",
    "bench.status",
    "bench.logs",
    "infra.plan",
    "ci.rerun",
    "ci.cloudflare",
    "ci.window",
    "sites.build",
    "sites.publish",
    "ios.release",
    "bench.run",
    "bench.cancel",
    "bench.reset",
    "bench.warm",
    "owner.ask",
    "owner.withdraw",
    "owner.answer",
    "provider.call",
    "infra.apply",
    "build.list",
    "build.read",
    "home.show",
    "tracker.show",
    "ops.show",
    "insights.show",
    "push.key",
    "push.subscribe",
];

/// Whether `op` is presented as a list, a detail or (otherwise) a card.
fn shape(op: &str) -> &'static str {
    match op {
        "git.tokens" | "bench.list" | "events.wait" | "provider.tools" | "feed.read"
        | "build.list" => "list",
        "instance.show"
        | "architecture.describe"
        | "policy.check"
        | "infra.validate"
        | "ios.status"
        | "apple.check"
        | "digest.show"
        | "bench.status"
        | "bench.logs"
        | "infra.plan"
        | "home.show"
        | "tracker.show"
        | "ops.show"
        | "insights.show" => "detail",
        _ => "card",
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
fn opt(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().map(str::to_string)
}

/// `v[k]` as a number, 0 when absent.
fn u(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}

/// `v[k]` as a count, 0 when absent.
fn n(v: &Value, k: &str) -> i64 {
    i64::try_from(u(v, k)).unwrap_or(i64::MAX)
}

/// The length of the array under `k`, as a count.
fn len(v: &Value, k: &str) -> i64 {
    i64::try_from(arr(v, k).len()).unwrap_or(i64::MAX)
}

/// The array under `k`, empty when absent.
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map_or(&[], Vec::as_slice)
}

/// `v[k]` when a string of 7 to 64 lowercase hex digits.
fn hex(v: &Value, k: &str) -> Option<String> {
    v[k].as_str()
        .filter(|x| {
            (7..=64).contains(&x.len())
                && x.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .map(str::to_string)
}

/// A boolean key written as text.
fn flag(v: &Value, k: &str) -> Value {
    text(&v[k].as_bool().unwrap_or(false).to_string())
}

/// `!<number>` from a number key.
fn bang(v: &Value, k: &str) -> String {
    format!("!{}", u(v, k))
}

/// The view of operation `op`'s result, when this family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<Value> {
    if !NAMES.contains(&op) {
        return None;
    }
    let v = &r.result;
    let mut out = if op == "architecture.describe" && v.is_string() {
        detail(
            "Architecture",
            vec![section(None, vec![], v.as_str())],
            vec![],
        )
    } else if !v.is_object() {
        fallback(op, v)
    } else {
        build(op, v)
    };
    if out["title"].as_str().is_none_or(|t| t.trim().is_empty()) {
        out["title"] = Value::String(op.to_string());
    }
    Some(out)
}

/// A result of the wrong shape, still as a valid view of its operation's kind.
fn fallback(op: &str, v: &Value) -> Value {
    match shape(op) {
        "list" => list(op, vec![], None),
        "detail" => detail(op, vec![section(None, scalars(v), None)], vec![]),
        _ => card(op, None, scalars(v), vec![]),
    }
}

fn build(op: &str, v: &Value) -> Value {
    match shape(op) {
        "list" => lists(op, v),
        "detail" => details(op, v),
        _ => cards(op, v),
    }
}

fn lists(op: &str, v: &Value) -> Value {
    match op {
        "git.tokens" => {
            let rows = arr(v, "tokens")
                .iter()
                .map(|t| {
                    let id = s(t, "id");
                    row(
                        &id,
                        &id,
                        opt(t, "scope").as_deref(),
                        opt(t, "state").as_deref(),
                        None,
                        None,
                    )
                })
                .collect();
            list("Git tokens", rows, None)
        }
        "bench.list" => {
            let rows = arr(v, "benches")
                .iter()
                .map(|b| {
                    let name = s(b, "name");
                    let badge = b["busy"].as_bool().unwrap_or(false).then_some("busy");
                    row(
                        &name,
                        &name,
                        Some(&format!("used {}", s(b, "used"))),
                        badge,
                        None,
                        None,
                    )
                })
                .collect();
            list("Benches", rows, None)
        }
        "events.wait" => {
            let rows = arr(v, "events")
                .iter()
                .map(|e| {
                    let badge = e["change"].is_number().then(|| bang(e, "change"));
                    let nd = e["stage"]
                        .is_object()
                        .then(|| needs(&s(&e["stage"], "needs")));
                    row(
                        &s(e, "id"),
                        &s(e, "summary"),
                        opt(e, "kind").as_deref(),
                        badge.as_deref(),
                        nd,
                        None,
                    )
                })
                .collect();
            list("Events", rows, None)
        }
        "build.list" => build_list(v),
        "provider.tools" => {
            let rows = arr(v, "tools")
                .iter()
                .map(|t| {
                    let name = s(t, "name");
                    let badge = t["drift"].as_bool().unwrap_or(false).then_some("drift");
                    row(&name, &name, None, badge, None, None)
                })
                .collect();
            list("Provider tools", rows, None)
        }
        _ => {
            let rows = arr(v, "items")
                .iter()
                .map(|i| {
                    let has = i["change"].is_number();
                    let badge = has.then(|| bang(i, "change"));
                    let open = has.then(|| {
                        action(
                            "Open",
                            "change.status",
                            serde_json::json!({ "number": u(i, "change") }),
                            "plain",
                            false,
                        )
                    });
                    row(
                        &s(i, "id"),
                        &s(i, "summary"),
                        opt(i, "type").as_deref(),
                        badge.as_deref(),
                        i["needs"].as_str().map(needs),
                        open,
                    )
                })
                .collect();
            list("Feed", rows, None)
        }
    }
}

/// `build.list`: a commit's stored build files, each with its size.
fn build_list(v: &Value) -> Value {
    let rows = arr(v, "files")
        .iter()
        .map(|f| {
            let name = s(f, "name");
            row(
                &name,
                &name,
                Some(&format!("{} bytes", u(f, "size"))),
                None,
                None,
                None,
            )
        })
        .collect();
    let sha = s(v, "sha");
    list(
        &format!("Build {}", sha.get(..12).unwrap_or(&sha)),
        rows,
        None,
    )
}

fn details(op: &str, v: &Value) -> Value {
    match op {
        "instance.show" => detail(
            &s(v, "name"),
            vec![section(
                None,
                vec![
                    field("description", text(&s(v, "description"))),
                    field("schema", text(&s(v, "schema"))),
                    field("repository", text(&s(v, "repository"))),
                ],
                None,
            )],
            vec![],
        ),
        "architecture.describe" => {
            let mut f = vec![
                field("nodes", count(len(v, "nodes"))),
                field("edges", count(len(v, "edges"))),
                field("groups", count(len(v, "groups"))),
            ];
            if let Some(d) = hex(v, "deployed") {
                f.push(field("deployed", sha(&d)));
            }
            detail("Architecture", vec![section(None, f, None)], vec![])
        }
        "policy.check" => policy_check(v),
        "infra.validate" => {
            let f = arr(v, "units")
                .iter()
                .enumerate()
                .map(|(i, u_)| field(&format!("{}", i + 1), code(&s(u_, "code"))))
                .collect();
            detail(
                "Infra validate",
                vec![section(Some("Units"), f, None)],
                vec![],
            )
        }
        "ios.status" => ios_status(v),
        "apple.check" => apple_check(v),
        "digest.show" => {
            let lines: Vec<&str> = arr(v, "lines").iter().filter_map(Value::as_str).collect();
            detail(
                "Digest",
                vec![section(
                    None,
                    vec![field("needs owner", flag(v, "needs_owner"))],
                    Some(&lines.join("\n")),
                )],
                vec![],
            )
        }
        "bench.status" | "bench.logs" => bench_detail(v),
        "home.show" | "tracker.show" | "ops.show" | "insights.show" => parts(op, v),
        _ => {
            let mut sections = vec![section(
                None,
                vec![field("expires", time(u(v, "expires")))],
                None,
            )];
            for unit in arr(v, "units") {
                let f = arr(unit, "changes")
                    .iter()
                    .map(|c| field(&s(c, "address"), text(&s(c, "action"))))
                    .collect();
                sections.push(section(Some(&s(unit, "unit")), f, None));
            }
            detail(&format!("Plan {}", s(v, "plan")), sections, vec![])
        }
    }
}

fn policy_check(v: &Value) -> Value {
    let mut sections = vec![section(
        None,
        vec![
            field("ran", count(n(v, "ran"))),
            field("skipped", count(len(v, "skipped"))),
        ],
        None,
    )];
    let violations: Vec<&str> = arr(v, "violations")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if !violations.is_empty() {
        sections.push(section(
            Some("Violations"),
            vec![],
            Some(&violations.join("\n")),
        ));
    }
    detail("Policy check", sections, vec![])
}

fn apple_check(v: &Value) -> Value {
    let f = v["checks"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, c)| {
                    let ok = if c["ok"].as_bool().unwrap_or(false) {
                        "ok"
                    } else {
                        "failed"
                    };
                    let t = match c["detail"].as_str() {
                        Some(d) => format!("{ok} · {d}"),
                        None => ok.to_string(),
                    };
                    field(k, text(&t))
                })
                .collect()
        })
        .unwrap_or_default();
    detail("Apple", vec![section(None, f, None)], vec![])
}

fn bench_detail(v: &Value) -> Value {
    let mut f = vec![field("state", text(&s(v, "state")))];
    if v["exit"].is_number() {
        f.push(field("exit", count(n(v, "exit"))));
    }
    f.push(field("seconds", duration(u(v, "seconds"))));
    if let Some(h) = hex(v, "sha") {
        f.push(field("sha", sha(&h)));
    }
    let log = v["log"].as_str().filter(|l| !l.is_empty());
    detail(
        &format!("Run {}", s(v, "run")),
        vec![section(None, f, log)],
        vec![],
    )
}

fn ios_status(v: &Value) -> Value {
    let mut sections = Vec::new();
    let cur = &v["in_flight"]["current"];
    let pending = &v["in_flight"]["pending"];
    if cur.is_object() || pending.is_string() {
        let mut f = Vec::new();
        if cur.is_object() {
            f.push(field("build", count(n(cur, "build"))));
            if let Some(h) = hex(cur, "sha") {
                f.push(field("sha", sha(&h)));
            }
            f.push(field("stage", text(&s(cur, "stage"))));
        }
        if let Some(p) = pending.as_str() {
            f.push(field("pending", text(p)));
        }
        sections.push(section(Some("In flight"), f, None));
    }
    let releases = arr(v, "releases")
        .iter()
        .take(10)
        .map(|r| field(&format!("#{}", s(r, "build")), text(&s(r, "state"))))
        .collect();
    sections.push(section(Some("Releases"), releases, None));
    detail("iOS releases", sections, vec![])
}

fn pending_fields(v: &Value) -> Vec<Value> {
    let p = &v["pending"];
    let mut f = vec![field("operation", text(&s(p, "operation")))];
    if p["number"].is_number() {
        f.push(field("change", text(&bang(p, "number"))));
    }
    if let Some(l) = p["link"].as_str() {
        f.push(field("approve", link("Approve with passkey", l)));
    }
    f
}

fn cards(op: &str, v: &Value) -> Value {
    match op {
        "ci.rerun" | "ci.cloudflare" => ci_card(op, v),
        "ci.window" => {
            let mut f = vec![
                field("hours", count(len(&v["offer"], "hours"))),
                field("expires", time(u(&v["offer"], "expires"))),
            ];
            if let Some(l) = v["pending"]["link"].as_str() {
                f.push(field("approve", link("Approve with passkey", l)));
            }
            card("CI window offered", None, f, vec![])
        }
        "sites.build" | "sites.publish" => sites_card(op, v),
        "ios.release" => {
            let mut f: Vec<Value> = hex(v, "sha")
                .map(|h| field("sha", sha(&h)))
                .into_iter()
                .collect();
            f.push(field("auto", flag(v, "auto")));
            card(
                &format!("iOS build {}", s(v, "build")),
                opt(v, "state").as_deref(),
                f,
                vec![],
            )
        }
        "bench.run" => {
            let mut f: Vec<Value> = hex(v, "sha")
                .map(|h| field("sha", sha(&h)))
                .into_iter()
                .collect();
            f.push(field("seconds", duration(u(v, "seconds"))));
            card(
                &format!("Run {}", s(v, "run")),
                opt(v, "state").as_deref(),
                f,
                vec![],
            )
        }
        "bench.cancel" => card(&format!("Cancelling {}", s(v, "run")), None, vec![], vec![]),
        "bench.reset" => card("Bench reset", None, scalars(v), vec![]),
        "bench.warm" => card("Bench", None, scalars(v), vec![]),
        "push.key" => card(
            "Notifications",
            None,
            vec![field("key", code(&s(v, "key")))],
            vec![],
        ),
        "push.subscribe" => card("This browser is notified", None, vec![], vec![]),
        "owner.ask" | "owner.withdraw" | "owner.answer" => owner_card(&v["item"]),
        "build.read" => build_read_card(v),
        "provider.call" => {
            if let Some(p) = v["pending"].as_str() {
                card("Pending", Some(p), vec![], vec![])
            } else if v["result"].is_object() {
                card("Provider call", None, scalars(&v["result"]), vec![])
            } else {
                card(
                    "Provider call",
                    None,
                    vec![field("result", code(&v["result"].to_string()))],
                    vec![],
                )
            }
        }
        _ => {
            let f = arr(v, "units")
                .iter()
                .map(|x| {
                    field(
                        &s(x, "unit"),
                        text(&format!("{} changes", arr(x, "changes").len())),
                    )
                })
                .collect();
            card(&format!("Applied {}", s(v, "plan")), None, f, vec![])
        }
    }
}

/// `build.read`: the piece's place in its file, never its data.
fn build_read_card(v: &Value) -> Value {
    let mut f = vec![
        field("length", count(n(v, "length"))),
        field("size", count(n(v, "size"))),
    ];
    if v["next"].is_number() {
        f.push(field("next", count(n(v, "next"))));
    }
    card(
        &format!("{} from {}", s(v, "name"), u(v, "offset")),
        None,
        f,
        vec![],
    )
}

fn ci_card(op: &str, v: &Value) -> Value {
    if v["change"].is_object() {
        let c = &v["change"];
        let f = hex(c, "head")
            .map(|h| field("head", sha(&h)))
            .into_iter()
            .collect();
        card(
            &format!("{} {}", bang(c, "number"), s(c, "title")),
            Some(&format!("state: {}", s(c, "state"))),
            f,
            vec![],
        )
    } else if v["pending"].is_object() {
        card("Waiting for the owner", None, pending_fields(v), vec![])
    } else {
        card(op, None, scalars(v), vec![])
    }
}

fn sites_card(op: &str, v: &Value) -> Value {
    let built = op == "sites.build";
    let f = arr(v, "sites")
        .iter()
        .map(|x| {
            let value = if built {
                s(x, "template")
            } else {
                format!(
                    "put {}, unchanged {}, deleted {}",
                    s(x, "put"),
                    s(x, "unchanged"),
                    s(x, "deleted")
                )
            };
            field(&s(x, "unit"), text(&value))
        })
        .collect();
    card(
        if built {
            "Sites built"
        } else {
            "Sites published"
        },
        None,
        f,
        vec![],
    )
}

fn owner_card(i: &Value) -> Value {
    let mut f = vec![field("kind", text(&s(i, "kind")))];
    if i["change"].is_number() {
        f.push(field("change", text(&bang(i, "change"))));
    }
    if i["options"].is_array() {
        let o: Vec<&str> = arr(i, "options").iter().filter_map(Value::as_str).collect();
        f.push(field("options", text(&o.join(", "))));
    }
    for k in ["answer", "by"] {
        if let Some(t) = i[k].as_str() {
            f.push(field(k, text(t)));
        }
    }
    card(
        &format!("#{} {}", s(i, "id"), s(i, "title")),
        i["detail"].as_str(),
        f,
        vec![],
    )
}

#[cfg(test)]
#[path = "rest_tests.rs"]
mod tests;

/// The app's reads: the result's scalars, then a section per object part and a count per list.
fn parts(op: &str, v: &Value) -> Value {
    let title = match op {
        "home.show" => "Home",
        "tracker.show" => "Tracker",
        "ops.show" => "Operations",
        _ => "Insights",
    };
    let mut top = scalars(v);
    let mut sections = Vec::new();
    if let Value::Object(o) = v {
        for (k, part) in o {
            match part {
                Value::Array(a) => {
                    top.push(field(k, count(i64::try_from(a.len()).unwrap_or(i64::MAX))));
                }
                Value::Object(_) => {
                    let f = scalars(part);
                    if !f.is_empty() {
                        sections.push(section(Some(k), f, None));
                    }
                }
                _ => {}
            }
        }
    }
    sections.insert(0, section(None, top, None));
    detail(title, sections, vec![])
}
