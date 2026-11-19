//! Each operation's declared result, which MCP advertises as its outputSchema and the presenters' test samples; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §C.1 and its 2026-11-18 amendment on 3b.

use super::{Arg, Kind, arg};
use serde_json::{Map, Value, json};

/// The fields of `name`'s result, if it declares them.
#[must_use]
pub fn result(name: &str) -> Option<&'static [Arg]> {
    rest(name)
        .or_else(|| change(name))
        .or_else(|| approval(name))
}

fn rest(name: &str) -> Option<&'static [Arg]> {
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

fn change(name: &str) -> Option<&'static [Arg]> {
    match name {
        "change.list" => Some(CHANGE_LIST),
        "change.evictable" => Some(EVICTABLE),
        "brief.list" => Some(BRIEF_LIST),
        "task.list" | "task.wait" => Some(TASKS),
        "change.scope" => Some(SCOPE),
        "session.status" => Some(SESSION_STATUS),
        "brief.show" => Some(BRIEF_SHOW),
        "change.status" => Some(CHANGE_STATUS),
        "change.propose" => Some(PROPOSED),
        "change.pin" => Some(PINNED),
        "change.front" => Some(FRONTED),
        "change.authorize" | "change.pushed" | "change.close" | "change.approve" => {
            Some(CHANGE_ACTED)
        }
        "change.merge" => Some(MERGED),
        "change.yield" => Some(YIELDED),
        "change.note" => Some(NOTED),
        "change.handoff" => Some(HANDOFF),
        "brief.create" | "brief.amend" => Some(BRIEF_MADE),
        "brief.review" => Some(BRIEF_REVIEW),
        "brief.escalate" => Some(BRIEF_ESCALATE),
        "session.start" => Some(SESSION_START),
        "session.end" => Some(SESSION_END),
        "session.archived" => Some(SESSION_ARCHIVED),
        "task.claim" | "task.done" => Some(TASK_TAKEN),
        _ => None,
    }
}

fn approval(name: &str) -> Option<&'static [Arg]> {
    match name {
        "approval.list" => Some(APPROVALS),
        "mandate.status" => Some(MANDATES),
        "platform.status" => Some(PLATFORM_RUNS),
        "platform.plan" => Some(PLATFORM_PLAN),
        "deploy.status" => Some(DEPLOY_STATUS),
        "approval.wait" => Some(APPROVAL_WAIT),
        "grant.request" | "grant.rotate" | "grant.revoke" => Some(PENDING_OP),
        "mandate.propose" => Some(MANDATE_PROPOSED),
        "mandate.end" => Some(MANDATE_ENDED),
        "lane.request" => Some(LANE_REQUESTED),
        "lane.status" => Some(LANES),
        "lane.end" => Some(LANE_ENDED),
        "platform.apply" => Some(PLATFORM_APPLY),
        "pool.resume" => Some(POOL_RESUME),
        "service.bind" | "service.unbind" => Some(SERVICE_BIND),
        "secrets.migrate" => Some(SECRETS_MIGRATE),
        "member.grant" | "member.revoke" => Some(MEMBER_GRANT),
        "deploy.retry" => Some(DEPLOY_RETRY),
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
const CH_STAGE: &[Arg] = &[
    arg("stage", false, Kind::String),
    arg("needs", false, Kind::String),
];
const CH_STAGE_FULL: &[Arg] = &[
    arg("stage", false, Kind::String),
    arg("needs", false, Kind::String),
    arg("next", false, Kind::String),
    arg("waits_on", false, Kind::Number),
    arg("since", false, Kind::Number),
];
const CHANGE_ROW: &[Arg] = &[
    arg("number", true, Kind::Number),
    arg("title", false, Kind::String),
    arg("state", false, Kind::String),
    arg("model", false, Kind::String),
    arg("stage", false, Kind::Object(CH_STAGE)),
];
const CHANGE_LIST: &[Arg] = &[arg("changes", false, Kind::List(CHANGE_ROW))];
const EVICT_FIELDS: &[Arg] = &[
    arg("state", false, Kind::String),
    arg("kind", false, Kind::String),
];
const EVICT_ROW: &[Arg] = &[
    arg("number", true, Kind::Number),
    arg("fields", false, Kind::Object(EVICT_FIELDS)),
];
const EVICTABLE: &[Arg] = &[arg("candidates", false, Kind::List(EVICT_ROW))];
const BRIEF_ROW: &[Arg] = &[
    arg("id", true, Kind::String),
    arg("title", false, Kind::String),
    arg("adr", false, Kind::String),
    arg("step", false, Kind::String),
    arg("model", false, Kind::String),
];
const BRIEF_LIST: &[Arg] = &[arg("briefs", false, Kind::List(BRIEF_ROW))];
const TASK_ROW: &[Arg] = &[
    arg("id", true, Kind::Number),
    arg("kind", false, Kind::String),
    arg("change", false, Kind::Number),
    arg("note", false, Kind::String),
    arg("claimed_by", false, Kind::String),
    arg("done", false, Kind::Bool),
];
const TASKS: &[Arg] = &[arg("tasks", false, Kind::List(TASK_ROW))];
const SCOPE_JOB: &[Arg] = &[
    arg("name", true, Kind::String),
    arg("argv", false, Kind::Strings),
];
const SCOPE: &[Arg] = &[arg("jobs", false, Kind::List(SCOPE_JOB))];
const LISTENING: &[Arg] = &[arg("listening", false, Kind::Bool)];
const SESSION_CHANGE: &[Arg] = &[
    arg("number", true, Kind::Number),
    arg("title", false, Kind::String),
    arg("stage", false, Kind::Object(CH_STAGE)),
];
const SESSION_TASK: &[Arg] = &[
    arg("id", true, Kind::Number),
    arg("kind", false, Kind::String),
    arg("note", false, Kind::String),
];
const SESSION_STATUS: &[Arg] = &[
    arg("session", false, Kind::String),
    arg("model", false, Kind::String),
    arg("role", false, Kind::String),
    arg("planner", false, Kind::String),
    arg("brief", false, Kind::String),
    arg("events", false, Kind::Object(LISTENING)),
    arg("changes", false, Kind::List(SESSION_CHANGE)),
    arg("tasks", false, Kind::List(SESSION_TASK)),
];
const BRIEF_STEP: &[Arg] = &[
    arg("do", true, Kind::String),
    arg("done_when", false, Kind::String),
];
const BRIEF_COMMIT: &[Arg] = &[
    arg("title", false, Kind::String),
    arg("plan", false, Kind::String),
];
const BRIEF_DRAFT: &[Arg] = &[
    arg("title", false, Kind::String),
    arg("adr", false, Kind::String),
    arg("step", false, Kind::String),
    arg("model", false, Kind::String),
    arg("base", false, Kind::String),
    arg("steps", false, Kind::List(BRIEF_STEP)),
    arg("decisions", false, Kind::Strings),
    arg("commit", false, Kind::Object(BRIEF_COMMIT)),
];
const BRIEF_REVISION: &[Arg] = &[arg("draft", false, Kind::Object(BRIEF_DRAFT))];
const BRIEF_SHOW: &[Arg] = &[
    arg("id", false, Kind::String),
    arg("revisions", false, Kind::List(BRIEF_REVISION)),
];
const STATUS_CHANGE: &[Arg] = &[
    arg("number", false, Kind::Number),
    arg("title", false, Kind::String),
    arg("state", false, Kind::String),
    arg("head", false, Kind::String),
    arg("plan", false, Kind::String),
    arg("jobs", false, Kind::List(STATUS_JOB)),
];
const STATUS_JOB: &[Arg] = &[
    arg("name", true, Kind::String),
    arg("status", false, Kind::String),
    arg("attempts", false, Kind::Number),
];
const CHANGE_STATUS: &[Arg] = &[
    arg("change", false, Kind::Object(STATUS_CHANGE)),
    arg("branch", false, Kind::String),
    arg("model", false, Kind::String),
    arg("approval", false, Kind::String),
    arg("position", false, Kind::Number),
    arg("stage", false, Kind::Object(CH_STAGE_FULL)),
];
const PROPOSED_CHANGE: &[Arg] = &[
    arg("state", false, Kind::String),
    arg("title", false, Kind::String),
];
const PROPOSED: &[Arg] = &[
    arg("number", false, Kind::Number),
    arg("branch", false, Kind::String),
    arg("change", false, Kind::Object(PROPOSED_CHANGE)),
    arg("overlaps", false, Kind::List(OPEN)),
];
const PINNED: &[Arg] = &[arg("pinned", false, Kind::Any)];
const FRONTED: &[Arg] = &[
    arg("fronted", false, Kind::Number),
    arg("message", false, Kind::String),
];
const ACTED_CHANGE: &[Arg] = &[
    arg("number", false, Kind::Number),
    arg("title", false, Kind::String),
    arg("state", false, Kind::String),
    arg("head", false, Kind::String),
];
const CHANGE_ACTED: &[Arg] = &[arg("change", false, Kind::Object(ACTED_CHANGE))];
const MERGED: &[Arg] = &[
    arg("change", false, Kind::Object(ACTED_CHANGE)),
    arg("message", false, Kind::String),
    arg("queued", false, Kind::Number),
];
const YIELDED: &[Arg] = &[
    arg("number", false, Kind::Number),
    arg("title", false, Kind::String),
    arg("state", false, Kind::String),
    arg("head", false, Kind::String),
];
const NOTED: &[Arg] = &[arg("noted", false, Kind::Number)];
const HANDOFF: &[Arg] = &[arg("offered", false, Kind::Number)];
const MADE_DRAFT: &[Arg] = &[arg("title", false, Kind::String)];
const MADE_REVISION: &[Arg] = &[arg("draft", false, Kind::Object(MADE_DRAFT))];
const BRIEF_ROUTE: &[Arg] = &[
    arg("route", false, Kind::String),
    arg("why", false, Kind::String),
];
const BRIEF_MADE: &[Arg] = &[
    arg("id", false, Kind::String),
    arg("revisions", false, Kind::List(MADE_REVISION)),
    arg("route", false, Kind::Object(BRIEF_ROUTE)),
];
const BRIEF_REVIEW: &[Arg] = &[
    arg("change", false, Kind::Number),
    arg("verdict", false, Kind::String),
    arg("brief", false, Kind::String),
];
const BRIEF_ESCALATE: &[Arg] = &[
    arg("change", false, Kind::Number),
    arg("escalated", false, Kind::String),
    arg("brief", false, Kind::String),
];
const SESSION_START: &[Arg] = &[
    arg("label", false, Kind::String),
    arg("model", false, Kind::String),
    arg("role", false, Kind::String),
    arg("planner", false, Kind::String),
    arg("brief", false, Kind::String),
    arg("expires", false, Kind::Number),
];
const HOLD_REMAINS: &[Arg] = &[arg("resume", false, Kind::String)];
const HOLD: &[Arg] = &[
    arg("number", true, Kind::Number),
    arg("title", false, Kind::String),
    arg("remains", false, Kind::Object(HOLD_REMAINS)),
];
const SESSION_END: &[Arg] = &[
    arg("label", false, Kind::String),
    arg("ended", false, Kind::String),
    arg("archiver", false, Kind::String),
    arg("clean", false, Kind::Bool),
    arg("holds", false, Kind::List(HOLD)),
];
const SESSION_ARCHIVED: &[Arg] = &[
    arg("outcome", false, Kind::String),
    arg("why", false, Kind::String),
    arg("by", false, Kind::String),
    arg("at", false, Kind::Number),
];
const TASK_TAKEN: &[Arg] = &[
    arg("id", false, Kind::Number),
    arg("kind", false, Kind::String),
    arg("change", false, Kind::Number),
    arg("note", false, Kind::String),
    arg("done", false, Kind::Bool),
    arg("closed", false, Kind::Number),
];
const APPROVAL_LANE: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("reason", false, Kind::String),
];
const APPROVAL_WAITING: &[Arg] = &[
    arg("path", true, Kind::String),
    arg("title", false, Kind::String),
    arg("operation", false, Kind::String),
    arg("expires", false, Kind::Number),
];
const APPROVALS: &[Arg] = &[
    arg("lanes", false, Kind::List(APPROVAL_LANE)),
    arg("waiting", false, Kind::List(APPROVAL_WAITING)),
];
const MANDATE_ROW: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("summary", false, Kind::String),
    arg("state", false, Kind::String),
    arg("used", false, Kind::Number),
    arg("budget", false, Kind::Number),
    arg("hours", false, Kind::Number),
    arg("expires", false, Kind::Number),
    arg("why", false, Kind::String),
];
const MANDATES: &[Arg] = &[arg("mandates", false, Kind::List(MANDATE_ROW))];
const RUN_ROW: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("unit", false, Kind::String),
    arg("stage", false, Kind::String),
    arg("created", false, Kind::Number),
    arg("sha", false, Kind::String),
    arg("why", false, Kind::String),
];
const PLATFORM_RUNS: &[Arg] = &[arg("runs", false, Kind::List(RUN_ROW))];
const PLAN_RUN: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("unit", false, Kind::String),
    arg("stage", false, Kind::String),
    arg("created", false, Kind::Number),
    arg("changes", false, Kind::Any),
];
const LINK_ONLY: &[Arg] = &[arg("link", false, Kind::String)];
const PLATFORM_PLAN: &[Arg] = &[
    arg("run", false, Kind::Object(PLAN_RUN)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const DEPLOY_HISTORY: &[Arg] = &[
    arg("sha", true, Kind::String),
    arg("outcome", false, Kind::String),
    arg("published", false, Kind::Strings),
];
const DEPLOY_PIPELINE: &[Arg] = &[
    arg("deployed", false, Kind::String),
    arg("target", false, Kind::String),
    arg("history", false, Kind::List(DEPLOY_HISTORY)),
];
const DEPLOY_MIRROR: &[Arg] = &[
    arg("mirrored", false, Kind::String),
    arg("failures", false, Kind::Number),
];
const MACOS_PROVIDER: &[Arg] = &[
    arg("provider", true, Kind::String),
    arg("health", false, Kind::String),
    arg("minutes", false, Kind::Number),
];
const DEPLOY_MACOS: &[Arg] = &[arg("providers", false, Kind::List(MACOS_PROVIDER))];
const DEPLOY_STATUS: &[Arg] = &[
    arg("pipeline", false, Kind::Object(DEPLOY_PIPELINE)),
    arg("mirror", false, Kind::Object(DEPLOY_MIRROR)),
    arg("mirror_behind", false, Kind::String),
    arg("macos", false, Kind::Object(DEPLOY_MACOS)),
];
const WAIT_PENDING: &[Arg] = &[
    arg("operation", false, Kind::String),
    arg("number", false, Kind::Text),
    arg("link", false, Kind::String),
];
const APPROVAL_WAIT: &[Arg] = &[
    arg("message", false, Kind::String),
    arg("pending", false, Kind::Object(WAIT_PENDING)),
];
const PENDING_OP_FIELDS: &[Arg] = &[
    arg("operation", false, Kind::String),
    arg("link", false, Kind::String),
];
const PENDING_OP: &[Arg] = &[arg("pending", false, Kind::Object(PENDING_OP_FIELDS))];
const MANDATE_SUMMARY: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("summary", false, Kind::String),
];
const MANDATE_PROPOSED: &[Arg] = &[
    arg("mandate", false, Kind::Object(MANDATE_SUMMARY)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const MANDATE_ENDED: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("state", false, Kind::String),
    arg("summary", false, Kind::String),
    arg("used", false, Kind::Number),
];
const LANE_SUMMARY: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("reason", false, Kind::String),
];
const LANE_REQUESTED: &[Arg] = &[
    arg("lane", false, Kind::Object(LANE_SUMMARY)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const LANE_ROW: &[Arg] = &[
    arg("id", true, Kind::Text),
    arg("state", false, Kind::String),
    arg("reason", false, Kind::String),
    arg("waived", false, Kind::List(OPEN)),
    arg("merged", false, Kind::Any),
];
const LANES: &[Arg] = &[arg("lanes", false, Kind::List(LANE_ROW))];
const LANE_ENDED: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("state", false, Kind::String),
    arg("reason", false, Kind::String),
    arg("waived", false, Kind::List(OPEN)),
    arg("merged", false, Kind::Any),
];
const APPLY_RUN: &[Arg] = &[
    arg("id", false, Kind::Text),
    arg("unit", false, Kind::String),
    arg("stage", false, Kind::String),
];
const PLATFORM_APPLY: &[Arg] = &[
    arg("run", false, Kind::Object(APPLY_RUN)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const RESUME_OFFER: &[Arg] = &[
    arg("pool", false, Kind::String),
    arg("expires", false, Kind::Number),
];
const POOL_RESUME: &[Arg] = &[
    arg("offer", false, Kind::Object(RESUME_OFFER)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const BIND_OFFER: &[Arg] = &[
    arg("account", false, Kind::String),
    arg("client_hash", false, Kind::String),
    arg("unbind", false, Kind::Bool),
    arg("expires", false, Kind::Number),
];
const SERVICE_BIND: &[Arg] = &[
    arg("offer", false, Kind::Object(BIND_OFFER)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const MIGRATE_OFFER: &[Arg] = &[
    arg("id", false, Kind::Number),
    arg("names", false, Kind::Strings),
    arg("expires", false, Kind::Number),
];
const SECRETS_MIGRATE: &[Arg] = &[
    arg("offer", false, Kind::Object(MIGRATE_OFFER)),
    arg("pending", false, Kind::Object(PENDING_OP_FIELDS)),
];
const GRANT_OFFER: &[Arg] = &[
    arg("account", false, Kind::String),
    arg("capability", false, Kind::String),
    arg("scope", false, Kind::String),
    arg("until", false, Kind::Number),
    arg("revoke", false, Kind::Bool),
    arg("expires", false, Kind::Number),
];
const MEMBER_GRANT: &[Arg] = &[
    arg("offer", false, Kind::Object(GRANT_OFFER)),
    arg("pending", false, Kind::Object(LINK_ONLY)),
];
const RETRY_PIPELINE: &[Arg] = &[arg("target", false, Kind::String)];
const DEPLOY_RETRY: &[Arg] = &[arg("pipeline", false, Kind::Object(RETRY_PIPELINE))];

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
