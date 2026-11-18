//! Presenters: each operation's result as its declared view, beside the result; see
//! .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md's 2026-11-17 amendment on presenting.

pub(crate) mod app;
pub(crate) mod approval;
pub mod asked;
pub(crate) mod change;
pub mod proof;
pub(crate) mod rest;
#[cfg(test)]
mod tests;

use serde_json::{Map, Value, json};

/// Every operation with a presenter.
#[must_use]
pub fn presented() -> Vec<&'static str> {
    let mut all = Vec::new();
    all.extend_from_slice(change::NAMES);
    all.extend_from_slice(app::NAMES);
    all.extend_from_slice(approval::NAMES);
    all.extend_from_slice(rest::NAMES);
    all
}

/// Operation `op`'s result as its view, when any family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<Value> {
    change::view(op, r)
        .or_else(|| approval::view(op, r))
        .or_else(|| rest::view(op, r))
        .or_else(|| app::view(op, r))
}

/// A view checked against the one its operation declares.
///
/// # Errors
/// The problems, when `v` is not a valid `declared` view.
pub fn checked(
    declared: crate::view::View,
    v: Option<Value>,
) -> Result<Option<Value>, Vec<String>> {
    match v {
        None => Ok(None),
        Some(v) => crate::view::validate(declared, &v).map(|()| Some(v)),
    }
}

/// A plain text value.
#[must_use]
#[allow(dead_code)]
pub(crate) fn text(s: &str) -> Value {
    json!({ "kind": "text", "text": s })
}

/// A code value.
#[must_use]
#[allow(dead_code)]
pub(crate) fn code(s: &str) -> Value {
    json!({ "kind": "code", "text": s })
}

/// A commit hash value.
#[must_use]
#[allow(dead_code)]
pub(crate) fn sha(s: &str) -> Value {
    json!({ "kind": "sha", "text": s })
}

/// A point in time, in seconds since the epoch.
#[must_use]
#[allow(dead_code)]
pub(crate) fn time(at: u64) -> Value {
    json!({ "kind": "time", "at": at })
}

/// A span of seconds.
#[must_use]
#[allow(dead_code)]
pub(crate) fn duration(seconds: u64) -> Value {
    json!({ "kind": "duration", "seconds": seconds })
}

/// A whole number.
#[must_use]
#[allow(dead_code)]
pub(crate) fn count(n: i64) -> Value {
    json!({ "kind": "count", "n": n })
}

/// A link, or plain text when `href` is neither `https://` nor `pigeon://`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn link(text_: &str, href: &str) -> Value {
    if href.starts_with("https://") || href.starts_with("pigeon://") {
        json!({ "kind": "link", "text": text_, "href": href })
    } else {
        text(&format!("{text_} {href}"))
    }
}

/// A progress value: `stop` of `of`, marked `mark`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn progress(stop: u64, of: u64, mark: &str) -> Value {
    json!({ "kind": "progress", "stop": stop, "of": of, "mark": mark })
}

/// A labelled value.
#[must_use]
#[allow(dead_code)]
pub(crate) fn field(label: &str, value: Value) -> Value {
    let mut o = Map::new();
    o.insert("label".into(), json!(label));
    o.insert("value".into(), value);
    Value::Object(o)
}

/// A call to a tool the person may make from the view.
#[must_use]
#[allow(dead_code)]
pub(crate) fn action(label: &str, tool: &str, arguments: Value, style: &str, proof: bool) -> Value {
    let mut o = Map::new();
    o.insert("label".into(), json!(label));
    o.insert("tool".into(), json!(tool));
    o.insert("arguments".into(), arguments);
    o.insert("style".into(), json!(style));
    o.insert("proof".into(), json!(proof));
    Value::Object(o)
}

/// One row of a list; absent options are left out.
#[must_use]
#[allow(dead_code)]
pub(crate) fn row(
    id: &str,
    title: &str,
    subtitle: Option<&str>,
    badge: Option<&str>,
    needs: Option<&str>,
    open: Option<Value>,
) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("title".into(), json!(title));
    if let Some(s) = subtitle {
        o.insert("subtitle".into(), json!(s));
    }
    if let Some(b) = badge {
        o.insert("badge".into(), json!(b));
    }
    if let Some(n) = needs {
        o.insert("needs".into(), json!(n));
    }
    if let Some(a) = open {
        o.insert("open".into(), a);
    }
    Value::Object(o)
}

/// One section of a detail; absent options are left out.
#[must_use]
#[allow(dead_code)]
pub(crate) fn section(heading: Option<&str>, fields: Vec<Value>, text_: Option<&str>) -> Value {
    let mut o = Map::new();
    if let Some(h) = heading {
        o.insert("heading".into(), json!(h));
    }
    o.insert("fields".into(), Value::Array(fields));
    if let Some(t) = text_ {
        o.insert("text".into(), json!(t));
    }
    Value::Object(o)
}

/// A card view.
#[must_use]
#[allow(dead_code)]
pub(crate) fn card(
    title: &str,
    body: Option<&str>,
    fields: Vec<Value>,
    actions: Vec<Value>,
) -> Value {
    let mut o = Map::new();
    o.insert("view".into(), json!("card"));
    o.insert("title".into(), json!(title));
    if let Some(b) = body {
        o.insert("body".into(), json!(b));
    }
    o.insert("fields".into(), Value::Array(fields));
    o.insert("actions".into(), Value::Array(actions));
    Value::Object(o)
}

/// A detail view; no sections becomes one section with no fields.
#[must_use]
#[allow(dead_code)]
pub(crate) fn detail(title: &str, sections: Vec<Value>, actions: Vec<Value>) -> Value {
    let sections = if sections.is_empty() {
        vec![section(None, Vec::new(), None)]
    } else {
        sections
    };
    let mut o = Map::new();
    o.insert("view".into(), json!("detail"));
    o.insert("title".into(), json!(title));
    o.insert("sections".into(), Value::Array(sections));
    o.insert("actions".into(), Value::Array(actions));
    Value::Object(o)
}

/// A list view.
#[must_use]
#[allow(dead_code)]
pub(crate) fn list(title: &str, items: Vec<Value>, next: Option<&str>) -> Value {
    let mut o = Map::new();
    o.insert("view".into(), json!("list"));
    o.insert("title".into(), json!(title));
    o.insert("items".into(), Value::Array(items));
    if let Some(n) = next {
        o.insert("next".into(), json!(n));
    }
    Value::Object(o)
}

/// A stage's `needs` as a row's: `owner`, `agent` or `none`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn needs(stage_needs: &str) -> &'static str {
    match stage_needs {
        "owner" => "owner",
        "agent" => "agent",
        _ => "none",
    }
}

/// A result's scalars as fields: an object's top-level scalars in key order, or a string as `result`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn scalars(v: &Value) -> Vec<Value> {
    match v {
        Value::Object(o) => o
            .iter()
            .filter_map(|(k, v)| {
                let value = match v {
                    Value::String(s) => text(s),
                    Value::Bool(b) => text(&b.to_string()),
                    Value::Number(n) => n.as_i64().map_or_else(|| text(&n.to_string()), count),
                    _ => return None,
                };
                Some(field(k, value))
            })
            .collect(),
        Value::String(s) => vec![field("result", text(s))],
        _ => Vec::new(),
    }
}
