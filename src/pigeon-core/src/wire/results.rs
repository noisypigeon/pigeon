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
        "events.wait" => Some(EVENTS),
        "provider.tools" => Some(TOOLS),
        "feed.read" => Some(FEED),
        "build.list" => Some(BUILDS),
        "instance.show" => Some(INSTANCE),
        "policy.check" => Some(POLICY),
        "infra.validate" => Some(VALIDATE),
        "ios.status" => Some(IOS_STATUS),
        "apple.check" => Some(APPLE),
        "digest.show" => Some(DIGEST),
        "bench.status" | "bench.logs" => Some(RUN),
        "infra.plan" => Some(PLAN),
        "ci.rerun" | "ci.cloudflare" => Some(CI),
        "ci.window" => Some(WINDOW),
        "sites.build" => Some(SITES_BUILD),
        "sites.publish" => Some(SITES_PUBLISH),
        "ios.release" => Some(IOS_RELEASED),
        "bench.run" => Some(BENCH_RUN),
        "bench.cancel" => Some(BENCH_CANCEL),
        "owner.ask" | "owner.withdraw" | "owner.answer" => Some(OWNER),
        "build.read" => Some(BUILD_READ),
        "infra.apply" => Some(APPLY),
        "push.key" => Some(PUSH_KEY),
        "push.subscribe" | "home.show" | "tracker.show" | "ops.show" | "insights.show" => {
            Some(OPEN)
        }
        _ => None,
    }
}

const OPEN: &[Arg] = &[];
const PUSH_KEY: &[Arg] = &[arg("key", false, Kind::String)];
const STAGE: &[Arg] = &[arg("needs", false, Kind::String)];
const EVENT: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("summary", false, Kind::String),
    arg("kind", false, Kind::String),
    arg("change", false, Kind::Number),
    arg("stage", false, Kind::Object(STAGE)),
];
const EVENTS: &[Arg] = &[arg("events", false, Kind::List(EVENT))];
const TOOL: &[Arg] = &[
    arg("name", true, Kind::String),
    arg("drift", false, Kind::Bool),
];
const TOOLS: &[Arg] = &[arg("tools", false, Kind::List(TOOL))];
const FEED_ITEM: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("summary", false, Kind::String),
    arg("type", false, Kind::String),
    arg("change", false, Kind::Number),
    arg("needs", false, Kind::String),
];
const FEED: &[Arg] = &[arg("items", false, Kind::List(FEED_ITEM))];
const BUILD_FILE: &[Arg] = &[
    arg("name", true, Kind::String),
    arg("size", false, Kind::Number),
];
const BUILDS: &[Arg] = &[
    arg("sha", false, Kind::String),
    arg("files", false, Kind::List(BUILD_FILE)),
];
const INSTANCE: &[Arg] = &[
    arg("name", false, Kind::String),
    arg("description", false, Kind::String),
    arg("schema", false, Kind::String),
    arg("repository", false, Kind::String),
];
const POLICY: &[Arg] = &[
    arg("ran", false, Kind::Number),
    arg("skipped", false, Kind::Strings),
    arg("violations", false, Kind::Strings),
];
const VALIDATE_UNIT: &[Arg] = &[arg("code", true, Kind::String)];
const VALIDATE: &[Arg] = &[arg("units", false, Kind::List(VALIDATE_UNIT))];
const CURRENT: &[Arg] = &[
    arg("build", false, Kind::Text),
    arg("sha", false, Kind::String),
    arg("stage", false, Kind::String),
];
const IN_FLIGHT: &[Arg] = &[
    arg("current", false, Kind::Object(CURRENT)),
    arg("pending", false, Kind::String),
];
const IOS_RELEASE: &[Arg] = &[
    arg("build", true, Kind::Text),
    arg("state", false, Kind::String),
];
const IOS_STATUS: &[Arg] = &[
    arg("in_flight", false, Kind::Object(IN_FLIGHT)),
    arg("releases", false, Kind::List(IOS_RELEASE)),
];
const APPLE: &[Arg] = &[arg("checks", false, Kind::Object(OPEN))];
const DIGEST: &[Arg] = &[
    arg("lines", false, Kind::Strings),
    arg("needs_owner", false, Kind::Bool),
];
const RUN: &[Arg] = &[
    arg("state", false, Kind::String),
    arg("exit", false, Kind::Number),
    arg("seconds", false, Kind::Text),
    arg("sha", false, Kind::String),
    arg("log", false, Kind::String),
    arg("run", false, Kind::String),
];
const PLAN_CHANGE: &[Arg] = &[
    arg("address", false, Kind::String),
    arg("action", false, Kind::String),
];
const PLAN_UNIT: &[Arg] = &[
    arg("unit", true, Kind::String),
    arg("changes", false, Kind::List(PLAN_CHANGE)),
];
const PLAN: &[Arg] = &[
    arg("expires", false, Kind::Text),
    arg("plan", false, Kind::String),
    arg("units", false, Kind::List(PLAN_UNIT)),
];
const CI_CHANGE: &[Arg] = &[
    arg("number", false, Kind::Number),
    arg("title", false, Kind::String),
    arg("state", false, Kind::String),
    arg("head", false, Kind::String),
];
const CI_PENDING: &[Arg] = &[
    arg("operation", false, Kind::String),
    arg("number", false, Kind::Number),
    arg("link", false, Kind::String),
];
const CI: &[Arg] = &[
    arg("change", false, Kind::Object(CI_CHANGE)),
    arg("pending", false, Kind::Object(CI_PENDING)),
];
const WINDOW_OFFER: &[Arg] = &[
    arg("hours", false, Kind::Strings),
    arg("expires", false, Kind::Text),
];
const WINDOW_PENDING: &[Arg] = &[arg("link", false, Kind::String)];
const WINDOW: &[Arg] = &[
    arg("offer", false, Kind::Object(WINDOW_OFFER)),
    arg("pending", false, Kind::Object(WINDOW_PENDING)),
];
const SITE_BUILT: &[Arg] = &[
    arg("unit", true, Kind::String),
    arg("template", false, Kind::String),
];
const SITES_BUILD: &[Arg] = &[arg("sites", false, Kind::List(SITE_BUILT))];
const SITE_PUT: &[Arg] = &[
    arg("unit", true, Kind::String),
    arg("put", false, Kind::Number),
    arg("unchanged", false, Kind::Number),
    arg("deleted", false, Kind::Number),
];
const SITES_PUBLISH: &[Arg] = &[arg("sites", false, Kind::List(SITE_PUT))];
const IOS_RELEASED: &[Arg] = &[
    arg("sha", false, Kind::String),
    arg("auto", false, Kind::Bool),
    arg("build", false, Kind::Text),
    arg("state", false, Kind::String),
];
const BENCH_RUN: &[Arg] = &[
    arg("sha", false, Kind::String),
    arg("seconds", false, Kind::Text),
    arg("run", false, Kind::String),
    arg("state", false, Kind::String),
];
const BENCH_CANCEL: &[Arg] = &[arg("run", false, Kind::String)];
const OWNER_ITEM: &[Arg] = &[
    arg("id", false, Kind::Number),
    arg("title", false, Kind::String),
    arg("detail", false, Kind::String),
    arg("kind", false, Kind::String),
    arg("change", false, Kind::Number),
    arg("options", false, Kind::Strings),
    arg("answer", false, Kind::String),
    arg("by", false, Kind::String),
];
const OWNER: &[Arg] = &[arg("item", false, Kind::Object(OWNER_ITEM))];
const BUILD_READ: &[Arg] = &[
    arg("length", false, Kind::Number),
    arg("size", false, Kind::Number),
    arg("next", false, Kind::Number),
    arg("name", false, Kind::String),
    arg("offset", false, Kind::Number),
];
const APPLY_UNIT: &[Arg] = &[
    arg("unit", true, Kind::String),
    arg("changes", false, Kind::List(OPEN)),
];
const APPLY: &[Arg] = &[
    arg("plan", false, Kind::String),
    arg("units", false, Kind::List(APPLY_UNIT)),
];

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
