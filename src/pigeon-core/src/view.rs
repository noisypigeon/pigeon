//! The rendering contract: the four views every tool result names, their values, inputs and actions; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §2.

use serde::Serialize;
use serde_json::{Map, Value};
use std::fmt::Write as _;

/// The contract's version, answered in initialize's _meta.
pub const CONTRACT: u32 = 1;

macro_rules! contract_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: [Self; [$($text),+].len()] = [$(Self::$variant),+];

            /// The variant's lowercase wire name.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            /// The variant a wire name names, by exact match.
            #[must_use]
            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

contract_enum! {
    /// A tool result's view.
    View {
        /// Rows, each openable.
        List => "list",
        /// One thing: a body, fields and actions.
        Card => "card",
        /// Sections of fields and text.
        Detail => "detail",
        /// Inputs and a submit action.
        Form => "form",
    }
}

contract_enum! {
    /// How a field's value is shown.
    ValueKind {
        /// Plain text.
        Text => "text",
        /// Monospaced text.
        Code => "code",
        /// A commit hash.
        Sha => "sha",
        /// A unix time.
        Time => "time",
        /// A span of seconds.
        Duration => "duration",
        /// An integer.
        Count => "count",
        /// A link.
        Link => "link",
        /// Done out of total.
        Progress => "progress",
        /// Labelled numbers.
        Series => "series",
    }
}

contract_enum! {
    /// A form input's kind.
    Input {
        /// Free text.
        Text => "text",
        /// One of the options.
        Choice => "choice",
        /// On or off.
        Toggle => "toggle",
        /// A number.
        Number => "number",
        /// A value kept out of the transcript.
        Secret => "secret",
        /// A picked photo, sent as base64 JPEG of at most 512 KiB; the owner's #36.
        Image => "image",
    }
}

contract_enum! {
    /// An action's weight.
    Style {
        /// The main action.
        Primary => "primary",
        /// An ordinary action.
        Plain => "plain",
        /// An action that destroys something.
        Destructive => "destructive",
    }
}

contract_enum! {
    /// Who acts next on a row.
    Needs {
        /// The owner.
        Owner => "owner",
        /// The agent.
        Agent => "agent",
        /// No one.
        None => "none",
    }
}

contract_enum! {
    /// A progress's state.
    Mark {
        /// Going well.
        Ok => "ok",
        /// Went wrong.
        Failed => "failed",
        /// Not moving.
        Stalled => "stalled",
        /// Finished.
        Done => "done",
    }
}

fn names<const N: usize>(all: [&'static str; N]) -> String {
    all.join("|")
}

fn enum_names(kind: &str) -> String {
    match kind {
        "needs" => names(Needs::ALL.map(Needs::name)),
        "mark" => names(Mark::ALL.map(Mark::name)),
        "style" => names(Style::ALL.map(Style::name)),
        "input" => names(Input::ALL.map(Input::name)),
        _ => names(ValueKind::ALL.map(ValueKind::name)),
    }
}

fn child(path: &str, key: &str) -> String {
    format!("{path}.{key}")
}

fn at(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

fn closed(o: &Map<String, Value>, allowed: &[&str], path: &str, out: &mut Vec<String>) {
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            out.push(format!("{}: not in the contract", child(path, k)));
        }
    }
}

fn missing(path: &str, k: &str, out: &mut Vec<String>) {
    out.push(format!("{}: missing", child(path, k)));
}

fn string(
    o: &Map<String, Value>,
    path: &str,
    k: &str,
    required: bool,
    non_empty: bool,
    out: &mut Vec<String>,
) -> Option<String> {
    match o.get(k) {
        None => {
            if required {
                missing(path, k, out);
            }
            None
        }
        Some(Value::String(s)) if !non_empty || !s.is_empty() => Some(s.clone()),
        Some(Value::String(_)) => {
            out.push(format!("{}: expected a non-empty string", child(path, k)));
            None
        }
        Some(_) => {
            out.push(format!("{}: expected a string", child(path, k)));
            None
        }
    }
}

fn unsigned(o: &Map<String, Value>, path: &str, k: &str, out: &mut Vec<String>) -> Option<u64> {
    match o.get(k) {
        None => {
            missing(path, k, out);
            None
        }
        Some(v) => {
            let n = v.as_u64();
            if n.is_none() {
                out.push(format!(
                    "{}: expected a non-negative integer",
                    child(path, k)
                ));
            }
            n
        }
    }
}

fn boolean(o: &Map<String, Value>, path: &str, k: &str, out: &mut Vec<String>) {
    match o.get(k) {
        None => missing(path, k, out),
        Some(Value::Bool(_)) => {}
        Some(_) => out.push(format!("{}: expected a boolean", child(path, k))),
    }
}

fn object<'a>(v: &'a Value, path: &str, out: &mut Vec<String>) -> Option<&'a Map<String, Value>> {
    let o = v.as_object();
    if o.is_none() {
        out.push(format!("{path}: expected an object"));
    }
    o
}

fn array<'a>(
    o: &'a Map<String, Value>,
    path: &str,
    k: &str,
    required: bool,
    non_empty: bool,
    out: &mut Vec<String>,
) -> Option<&'a Vec<Value>> {
    match o.get(k) {
        None => {
            if required {
                missing(path, k, out);
            }
            None
        }
        Some(Value::Array(a)) => {
            if non_empty && a.is_empty() {
                out.push(format!("{}: expected a non-empty array", child(path, k)));
            }
            Some(a)
        }
        Some(_) => {
            out.push(format!("{}: expected an array", child(path, k)));
            None
        }
    }
}

fn each<F: FnMut(&Value, &str, &mut Vec<String>)>(
    items: Option<&Vec<Value>>,
    path: &str,
    k: &str,
    out: &mut Vec<String>,
    mut f: F,
) {
    if let Some(a) = items {
        let base = child(path, k);
        for (i, v) in a.iter().enumerate() {
            f(v, &at(&base, i), out);
        }
    }
}

fn enum_string(
    o: &Map<String, Value>,
    path: &str,
    k: &str,
    kind: &str,
    required: bool,
    out: &mut Vec<String>,
) -> Option<String> {
    let Some(v) = o.get(k) else {
        if required {
            missing(path, k, out);
        }
        return None;
    };
    let ok = v.as_str().is_some_and(|s| match kind {
        "needs" => Needs::parse(s).is_some(),
        "mark" => Mark::parse(s).is_some(),
        "style" => Style::parse(s).is_some(),
        "input" => Input::parse(s).is_some(),
        _ => ValueKind::parse(s).is_some(),
    });
    if ok {
        v.as_str().map(str::to_string)
    } else {
        out.push(format!("{}: expected {}", child(path, k), enum_names(kind)));
        None
    }
}

fn progress(o: &Map<String, Value>, path: &str, out: &mut Vec<String>) {
    let stop = unsigned(o, path, "stop", out);
    let of = unsigned(o, path, "of", out);
    if of == Some(0) {
        out.push(format!("{}: expected above 0", child(path, "of")));
    } else if let (Some(stop), Some(of)) = (stop, of)
        && stop > of
    {
        out.push(format!("{}: above of", child(path, "stop")));
    }
    enum_string(o, path, "mark", "mark", true, out);
}

fn progress_value(v: &Value, path: &str, out: &mut Vec<String>) {
    if let Some(o) = object(v, path, out) {
        closed(o, &["stop", "of", "mark"], path, out);
        progress(o, path, out);
    }
}

fn is_tool_name(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
}

fn action(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    closed(
        o,
        &["label", "tool", "arguments", "style", "proof"],
        path,
        out,
    );
    string(o, path, "label", true, false, out);
    if let Some(t) = string(o, path, "tool", true, false, out)
        && !is_tool_name(&t)
    {
        out.push(format!("{}: not a tool name", child(path, "tool")));
    }
    match o.get("arguments") {
        None => missing(path, "arguments", out),
        Some(Value::Object(_)) => {}
        Some(_) => out.push(format!("{}: expected an object", child(path, "arguments"))),
    }
    enum_string(o, path, "style", "style", true, out);
    boolean(o, path, "proof", out);
}

fn actions(o: &Map<String, Value>, path: &str, out: &mut Vec<String>) {
    each(
        array(o, path, "actions", false, false, out),
        path,
        "actions",
        out,
        action,
    );
}

fn value(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    let Some(kind) = enum_string(o, path, "kind", "kind", true, out) else {
        return;
    };
    match ValueKind::parse(&kind) {
        Some(ValueKind::Text | ValueKind::Code) => {
            closed(o, &["kind", "text"], path, out);
            string(o, path, "text", true, false, out);
        }
        Some(ValueKind::Sha) => {
            closed(o, &["kind", "text"], path, out);
            if let Some(t) = string(o, path, "text", true, false, out) {
                let hex = t.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
                if !(7..=64).contains(&t.len()) || !hex {
                    out.push(format!(
                        "{}: expected 7 to 64 lowercase hex digits",
                        child(path, "text")
                    ));
                }
            }
        }
        Some(ValueKind::Link) => {
            closed(o, &["kind", "text", "href"], path, out);
            string(o, path, "text", true, false, out);
            if let Some(h) = string(o, path, "href", true, false, out)
                && !h.starts_with("https://")
                && !h.starts_with("pigeon://")
            {
                out.push(format!(
                    "{}: expected a link starting https:// or pigeon://",
                    child(path, "href")
                ));
            }
        }
        Some(ValueKind::Time) => {
            closed(o, &["kind", "at"], path, out);
            unsigned(o, path, "at", out);
        }
        Some(ValueKind::Duration) => {
            closed(o, &["kind", "seconds"], path, out);
            unsigned(o, path, "seconds", out);
        }
        Some(ValueKind::Count) => {
            closed(o, &["kind", "n"], path, out);
            match o.get("n") {
                None => missing(path, "n", out),
                Some(n) if n.is_i64() || n.is_u64() => {}
                Some(_) => out.push(format!("{}: expected an integer", child(path, "n"))),
            }
        }
        Some(ValueKind::Progress) => {
            closed(o, &["kind", "stop", "of", "mark"], path, out);
            progress(o, path, out);
        }
        Some(ValueKind::Series) => {
            closed(o, &["kind", "points"], path, out);
            each(
                array(o, path, "points", true, false, out),
                path,
                "points",
                out,
                |p, pp, out| {
                    let Some(po) = object(p, pp, out) else {
                        return;
                    };
                    closed(po, &["label", "n"], pp, out);
                    string(po, pp, "label", true, false, out);
                    match po.get("n") {
                        None => missing(pp, "n", out),
                        Some(Value::Number(_)) => {}
                        Some(_) => out.push(format!("{}: expected a number", child(pp, "n"))),
                    }
                },
            );
        }
        None => {}
    }
}

fn field(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    closed(o, &["label", "value"], path, out);
    string(o, path, "label", true, false, out);
    match o.get("value") {
        None => missing(path, "value", out),
        Some(val) => value(val, &child(path, "value"), out),
    }
}

fn fields(o: &Map<String, Value>, path: &str, required: bool, out: &mut Vec<String>) {
    each(
        array(o, path, "fields", required, false, out),
        path,
        "fields",
        out,
        field,
    );
}

fn row(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    closed(
        o,
        &[
            "id", "title", "subtitle", "badge", "needs", "progress", "open",
        ],
        path,
        out,
    );
    string(o, path, "id", true, true, out);
    string(o, path, "title", true, false, out);
    string(o, path, "subtitle", false, false, out);
    string(o, path, "badge", false, false, out);
    enum_string(o, path, "needs", "needs", false, out);
    if let Some(p) = o.get("progress") {
        progress_value(p, &child(path, "progress"), out);
    }
    if let Some(a) = o.get("open") {
        action(a, &child(path, "open"), out);
    }
}

fn section(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    closed(o, &["heading", "fields", "text"], path, out);
    string(o, path, "heading", false, false, out);
    fields(o, path, true, out);
    string(o, path, "text", false, false, out);
}

fn form_input(v: &Value, path: &str, seen: &mut Vec<String>, out: &mut Vec<String>) {
    let Some(o) = object(v, path, out) else {
        return;
    };
    let kind = o
        .get("input")
        .and_then(Value::as_str)
        .and_then(Input::parse);
    let allowed: &[&str] = if kind == Some(Input::Choice) {
        &["key", "label", "input", "required", "help", "options"]
    } else {
        &["key", "label", "input", "required", "help"]
    };
    closed(o, allowed, path, out);
    if let Some(k) = string(o, path, "key", true, true, out) {
        if seen.contains(&k) {
            out.push(format!("{}: repeated", child(path, "key")));
        }
        seen.push(k);
    }
    string(o, path, "label", true, false, out);
    enum_string(o, path, "input", "input", true, out);
    boolean(o, path, "required", out);
    string(o, path, "help", false, false, out);
    if kind == Some(Input::Choice) {
        let options = array(o, path, "options", true, true, out);
        each(options, path, "options", out, |opt, p, out| {
            if !opt.is_string() {
                out.push(format!("{p}: expected a string"));
            }
        });
    }
}

/// Every way `v` departs from the contract's `view`, each as `<path>: <message>`; empty `Ok` when none.
///
/// # Errors
/// The problems, all of them, when `v` is not a valid `view`.
pub fn validate(view: View, v: &Value) -> Result<(), Vec<String>> {
    let mut out = Vec::new();
    let path = "$";
    if let Some(o) = object(v, path, &mut out) {
        match o.get("view") {
            None => missing(path, "view", &mut out),
            Some(Value::String(s)) if s == view.name() => {}
            Some(_) => out.push(format!("{}: expected {}", child(path, "view"), view.name())),
        }
        string(o, path, "title", true, true, &mut out);
        match view {
            View::List => {
                closed(o, &["view", "title", "items", "next"], path, &mut out);
                each(
                    array(o, path, "items", true, false, &mut out),
                    path,
                    "items",
                    &mut out,
                    row,
                );
                string(o, path, "next", false, false, &mut out);
            }
            View::Card => {
                closed(
                    o,
                    &["view", "title", "body", "fields", "actions"],
                    path,
                    &mut out,
                );
                string(o, path, "body", false, false, &mut out);
                fields(o, path, false, &mut out);
                actions(o, path, &mut out);
            }
            View::Detail => {
                closed(o, &["view", "title", "sections", "actions"], path, &mut out);
                each(
                    array(o, path, "sections", true, true, &mut out),
                    path,
                    "sections",
                    &mut out,
                    section,
                );
                actions(o, path, &mut out);
            }
            View::Form => {
                closed(o, &["view", "title", "fields", "submit"], path, &mut out);
                let mut seen = Vec::new();
                each(
                    array(o, path, "fields", true, true, &mut out),
                    path,
                    "fields",
                    &mut out,
                    |f, p, out| form_input(f, p, &mut seen, out),
                );
                match o.get("submit") {
                    None => missing(path, "submit", &mut out),
                    Some(a) => action(a, &child(path, "submit"), &mut out),
                }
            }
        }
    }
    if out.is_empty() { Ok(()) } else { Err(out) }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn render_action(a: &Value) -> String {
    let args = a
        .get("arguments")
        .map_or_else(|| "{}".to_string(), Value::to_string);
    let passkey = if a.get("proof").and_then(Value::as_bool) == Some(true) {
        " (passkey)"
    } else {
        ""
    };
    format!(
        "[{}] {} {}{}",
        str_of(a, "label"),
        str_of(a, "tool"),
        args,
        passkey
    )
}

fn render_progress(p: &Value) -> String {
    let n = |k: &str| p.get(k).and_then(Value::as_u64).unwrap_or(0);
    format!("{}/{} {}", n("stop"), n("of"), str_of(p, "mark"))
}

fn render_duration(s: u64) -> String {
    if s >= 3600 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

fn render_value(v: &Value) -> String {
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    match ValueKind::parse(str_of(v, "kind")) {
        Some(ValueKind::Text | ValueKind::Code) => str_of(v, "text").to_string(),
        Some(ValueKind::Sha) => str_of(v, "text").chars().take(12).collect(),
        Some(ValueKind::Link) => format!("{} <{}>", str_of(v, "text"), str_of(v, "href")),
        Some(ValueKind::Time) => {
            let t = crate::insights::cloudflare::rfc3339(n("at"));
            let head: String = t.chars().take(16).collect();
            format!("{} UTC", head.replace('T', " "))
        }
        Some(ValueKind::Duration) => render_duration(n("seconds")),
        Some(ValueKind::Count) => v.get("n").map_or_else(String::new, Value::to_string),
        Some(ValueKind::Progress) => render_progress(v),
        Some(ValueKind::Series) => v
            .get("points")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| {
                        let n = p.get("n").map_or_else(String::new, Value::to_string);
                        format!("{} {}", str_of(p, "label"), n)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn render_fields(v: &Value, lines: &mut Vec<String>) {
    for f in v
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let val = f.get("value").map(render_value).unwrap_or_default();
        lines.push(format!("{}: {}", str_of(f, "label"), val));
    }
}

fn render_actions(v: &Value, lines: &mut Vec<String>) {
    for a in v
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        lines.push(format!("- {}", render_action(a)));
    }
}

fn render_row(r: &Value) -> String {
    let mut s = format!("- {}", str_of(r, "title"));
    if let Some(t) = r.get("subtitle").and_then(Value::as_str) {
        let _ = write!(s, " · {t}");
    }
    if let Some(b) = r.get("badge").and_then(Value::as_str) {
        let _ = write!(s, " [{b}]");
    }
    let needs = str_of(r, "needs");
    if needs == "owner" || needs == "agent" {
        let _ = write!(s, " (needs {needs})");
    }
    if let Some(p) = r.get("progress") {
        let _ = write!(s, " {}", render_progress(p));
    }
    if let Some(a) = r.get("open") {
        let _ = write!(s, " → {}", render_action(a));
    }
    s
}

fn render_input(i: &Value) -> String {
    let required = if i.get("required").and_then(Value::as_bool) == Some(true) {
        ", required"
    } else {
        ""
    };
    let mut s = format!(
        "- {} ({}, {}{})",
        str_of(i, "label"),
        str_of(i, "key"),
        str_of(i, "input"),
        required
    );
    if let Some(h) = i.get("help").and_then(Value::as_str) {
        let _ = write!(s, " — {h}");
    }
    if str_of(i, "input") == "choice" {
        let opts: Vec<&str> = i
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let _ = write!(s, " [{}]", opts.join(", "));
    }
    s
}

/// The harness's text rendering of a view that validates; renders what it can of anything else.
#[must_use]
pub fn text(v: &Value) -> String {
    let mut lines = vec![format!("# {}", str_of(v, "title"))];
    match View::parse(str_of(v, "view")) {
        Some(View::List) => {
            for r in v
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                lines.push(render_row(r));
            }
            if let Some(n) = v.get("next").and_then(Value::as_str) {
                lines.push(format!("next: {n}"));
            }
        }
        Some(View::Card) => {
            if let Some(b) = v.get("body").and_then(Value::as_str) {
                lines.push(b.to_string());
            }
            render_fields(v, &mut lines);
            render_actions(v, &mut lines);
        }
        Some(View::Detail) => {
            for s in v
                .get("sections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(h) = s.get("heading").and_then(Value::as_str) {
                    lines.push(format!("## {h}"));
                }
                render_fields(s, &mut lines);
                if let Some(t) = s.get("text").and_then(Value::as_str) {
                    lines.push(t.to_string());
                }
            }
            render_actions(v, &mut lines);
        }
        Some(View::Form) => {
            for i in v
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                lines.push(render_input(i));
            }
            if let Some(a) = v.get("submit") {
                lines.push(format!("submit: {}", render_action(a)));
            }
        }
        None => {}
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests;
