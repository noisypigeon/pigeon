//! Each operation's declared result, which MCP advertises as its outputSchema and the presenters' test samples; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §C.1 and its 2026-11-18 amendment on 3b.

use super::{Arg, Kind, arg};
use serde_json::{Map, Value, json};

/// The fields of `name`'s result, if it declares them.
#[must_use]
pub fn result(name: &str) -> Option<&'static [Arg]> {
    const TOKEN: &[Arg] = &[
        arg("id", true, Kind::String),
        arg("scope", false, Kind::String),
        arg("state", false, Kind::String),
    ];
    const TOKENS: &[Arg] = &[arg("tokens", false, Kind::List(TOKEN))];
    const BENCH: &[Arg] = &[
        arg("name", true, Kind::String),
        arg("used", false, Kind::Text),
        arg("busy", false, Kind::Bool),
    ];
    const BENCHES: &[Arg] = &[arg("benches", false, Kind::List(BENCH))];
    match name {
        "git.tokens" => Some(TOKENS),
        "bench.list" => Some(BENCHES),
        _ => None,
    }
}

/// A result built from `fields`, every field present: text `x`, number 1, `true`, one-item lists.
#[must_use]
pub fn sample(fields: &[Arg]) -> Value {
    let map: Map<String, Value> = fields
        .iter()
        .map(|a| {
            let v = match a.kind {
                Kind::String | Kind::Text | Kind::Any => json!("x"),
                Kind::Number => json!(1),
                Kind::Bool => json!(true),
                Kind::Strings => json!(["x"]),
                Kind::List(f) => json!([sample(f)]),
                Kind::Object(f) => sample(f),
            };
            (a.name.to_string(), v)
        })
        .collect();
    Value::Object(map)
}
