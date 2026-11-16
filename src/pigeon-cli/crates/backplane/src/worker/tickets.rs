//! Session tickets in the control plane: `session.start`, the `Instance` object's record of
//! them, and the guard on agents' operations; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §5, and, for the
//! class a session declares, `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §1.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, now};
use super::to_change;
use crate::tickets::{Tickets, guarded, require, sign};
use crate::wire;

/// The `Instance` storage key the tickets live under.
pub const TICKETS: &str = "tickets";

/// The header a push through the git front door carries its ticket in.
pub const HEADER: &str = "x-pigeon-ticket";

fn key(env: &Env) -> Result<Vec<u8>> {
    Ok(platform::secret(env, "SESSION_KEY")?.into_bytes())
}

/// `session.start`: a ticket for `label`, replacing a live one only when asked.
///
/// # Errors
///
/// When a binding fails.
pub async fn start(
    env: &Env,
    s: &Settings,
    args: &serde_json::Map<String, Value>,
) -> Result<(u16, Value)> {
    let label = match crate::sessions::session(args.get("label").and_then(Value::as_str)) {
        Ok(Some(l)) => l,
        Ok(None) => return Ok((400, json!("`label` names the session, e.g. its URL"))),
        Err(why) => return Ok((400, json!(why))),
    };
    let replace = matches!(args.get("replace"), Some(Value::Bool(true)))
        || args.get("replace").and_then(Value::as_str) == Some("true");
    let text = |k: &str| args.get(k).and_then(Value::as_str);
    let declared = match crate::sessions::declare(text("model"), text("planner"), text("brief")) {
        Ok(d) => d,
        Err(why) => return Ok((400, json!(why))),
    };
    let Ok(parent) = crate::sessions::session(text("parent")) else {
        return Ok((
            400,
            json!("`parent` is the label of the session that created this one"),
        ));
    };
    let (status, v) = to_instance(
        env,
        s,
        "/tickets/start",
        &json!({ "label": label, "replace": replace, "declared": declared, "parent": parent }),
    )
    .await?;
    if status != 200 {
        return Ok((409, v));
    }
    let generation =
        u32::try_from(v["generation"].as_u64().unwrap_or_default()).unwrap_or_default();
    let expires = v["expires"].as_u64().unwrap_or_default();
    let kind = if generation > 1 {
        "session.replaced"
    } else {
        "session.started"
    };
    let class = declared.model;
    let said = format!(
        "session {label} has ticket {generation}, as {} ({})",
        class.name(),
        class.role()
    );
    super::events::append(env, s, kind, None, Some(label.clone()), &said, false).await?;
    Ok((
        200,
        json!({
            "label": label,
            "ticket": sign(&key(env)?, &label, generation, expires),
            "expires": expires,
            "model": class,
            "role": class.role(),
            "planner": declared.planner,
            "brief": declared.brief,
        }),
    ))
}

/// `session.end`: revokes `label`'s ticket; open to anyone, since it only takes power away.
///
/// # Errors
///
/// When a binding fails.
pub async fn end(
    env: &Env,
    s: &Settings,
    args: &serde_json::Map<String, Value>,
) -> Result<(u16, Value)> {
    let Some(label) = args
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok((400, json!("`label` names the session")));
    };
    let (_, v) = to_instance(env, s, "/tickets/end", &json!({ "label": label })).await?;
    let holds = held(env, s, &label).await?;
    let mut clean = false;
    let mut archiver = Value::Null;
    if v["ended"] == true {
        let numbers: Vec<u32> = holds
            .iter()
            .filter_map(|h| h["number"].as_u64().and_then(|n| u32::try_from(n).ok()))
            .collect();
        let (_, t) = to_instance(env, s, "/tasks/list", &json!({})).await?;
        let tasks: Vec<crate::tasks::Task> =
            serde_json::from_value(t["tasks"].clone()).unwrap_or_default();
        clean = crate::tickets::clean(&label, &numbers, &tasks);
        let (_, e) = to_instance(
            env,
            s,
            "/tickets/ended",
            &json!({ "label": label, "clean": clean }),
        )
        .await?;
        archiver = e["archiver"].clone();
        let how = if clean { "clean" } else { "with work left" };
        let said = format!("session {label}'s ticket ended, {how}");
        super::events::append(
            env,
            s,
            "session.ended",
            None,
            Some(label.clone()),
            &said,
            false,
        )
        .await?;
        if let (true, Some(by)) = (clean, archiver.as_str()) {
            let said = format!(
                "session {label} ended clean; archive {} (`session_archived` after)",
                crate::sessions::session_id(&label)
            );
            super::events::append(
                env,
                s,
                "session.archivable",
                None,
                Some(by.to_string()),
                &said,
                false,
            )
            .await?;
        }
    }
    Ok((
        200,
        json!({ "label": label, "ended": v["ended"], "holds": holds, "clean": clean, "archiver": archiver }),
    ))
}

/// Ends each live ticket whose session is silent past [`crate::tickets::IDLE`] holding no open
/// change and no claimed task; see the archive ADR's 2026-11-16 amendment.
///
/// # Errors
///
/// When a binding fails.
pub async fn lapse(env: &Env, s: &Settings) -> Result<()> {
    let (_, t) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: Tickets = serde_json::from_value(t).unwrap_or_default();
    let now = now();
    let rows = super::platform::d1_rows(env, &crate::control::open(&s.instance, &s.repo)).await?;
    let mut busy: std::collections::BTreeSet<String> = crate::control::open_rows(&rows)
        .into_iter()
        .filter_map(|o| o.session)
        .collect();
    let (_, t) = to_instance(env, s, "/tasks/list", &json!({})).await?;
    let tasks: Vec<crate::tasks::Task> =
        serde_json::from_value(t["tasks"].clone()).unwrap_or_default();
    busy.extend(
        tasks
            .into_iter()
            .filter(|t| !t.done)
            .filter_map(|t| t.claimed_by),
    );
    let mut polled = std::collections::BTreeMap::new();
    for label in tickets.0.keys().filter(|l| tickets.live(l, now).is_some()) {
        let (_, c) = to_instance(env, s, "/events/cursor", &json!({ "session": label })).await?;
        if let Some(p) = c["polled"].as_u64() {
            polled.insert(label.clone(), p);
        }
    }
    let lapsing = tickets.lapsing(now, &polled, &busy);
    if lapsing.is_empty() {
        return Ok(());
    }
    let (_, v) = to_instance(env, s, "/tickets/lapse", &json!({ "labels": lapsing })).await?;
    for label in v["lapsed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let quiet = polled.get(label).map_or_else(
            || "never polled".to_string(),
            |p| format!("silent {} min", now.saturating_sub(*p) / 60),
        );
        let said = format!("session {label}'s ticket lapsed: {quiet}, holding nothing");
        super::events::append(
            env,
            s,
            "session.ended",
            None,
            Some(label.to_string()),
            &said,
            false,
        )
        .await?;
    }
    Ok(())
}

/// `session.archived`: archiver `session`'s report on ended `label`; see
/// `.claude/decisions/2026-11-15-development-archive-cleanly-ended-sessions.md` §3.
///
/// # Errors
///
/// When a binding fails.
pub async fn archived(
    env: &Env,
    s: &Settings,
    args: &serde_json::Map<String, Value>,
) -> Result<(u16, Value)> {
    let text = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
    };
    let (label, by, outcome, why) = (text("label"), text("session"), text("outcome"), text("why"));
    if label.is_empty() || by.is_empty() {
        return Ok((
            400,
            json!("`label` names the ended session, `session` its archiver"),
        ));
    }
    let body = json!({ "label": label, "by": by, "outcome": outcome, "why": why });
    let (status, v) = to_instance(env, s, "/tickets/archived", &body).await?;
    if status != 200 {
        return Ok((409, v));
    }
    let id = crate::sessions::session_id(label);
    let said = format!(
        "session {id} {outcome} by {}: {why}",
        crate::sessions::session_id(by)
    );
    super::events::append(
        env,
        s,
        "session.archived",
        None,
        Some(label.to_string()),
        &said,
        false,
    )
    .await?;
    Ok((200, v))
}

/// Each open change `label` holds: its number, title and what it has left; see
/// `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §6.
async fn held(env: &Env, s: &Settings, label: &str) -> Result<Vec<Value>> {
    let rows = super::platform::d1_rows(env, &crate::control::open(&s.instance, &s.repo)).await?;
    let mut out = Vec::new();
    for o in crate::control::open_rows(&rows) {
        if o.session.as_deref() != Some(label) {
            continue;
        }
        let (status, v) =
            super::to_change(env, &s.repo, o.number, worker::Method::Get, "/status", None).await?;
        let Some(c) = (status == 200)
            .then(|| serde_json::from_value::<crate::changes::Change>(v["change"].clone()).ok())
            .flatten()
        else {
            continue;
        };
        out.push(json!({
            "number": c.number,
            "title": c.title,
            "remains": crate::core::change::eviction::remains(&c),
        }));
    }
    Ok(out)
}

/// Whether `ticket` lets an agent act for `labels` now.
///
/// # Errors
///
/// When a binding fails.
pub async fn allows(
    env: &Env,
    s: &Settings,
    labels: &[String],
    ticket: Option<&str>,
) -> Result<std::result::Result<(), String>> {
    if labels.is_empty() {
        return Ok(Ok(()));
    }
    let (_, v) = to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: Tickets = serde_json::from_value(v).unwrap_or_default();
    Ok(require(&key(env)?, &tickets, labels, ticket, now()))
}

/// The guard on an agent's changing `request`: the sessions it names, by argument or by its
/// change, must be acted on with their ticket.
///
/// # Errors
///
/// When a binding fails.
pub async fn guard(
    env: &Env,
    s: &Settings,
    request: &wire::Request,
) -> Result<std::result::Result<(), String>> {
    let text = |k: &str| {
        request
            .args
            .get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let number = request.args.get("number").and_then(|v| match v {
        Value::String(n) => n.parse::<u32>().ok(),
        other => other.as_u64().and_then(|n| u32::try_from(n).ok()),
    });
    let scoped = request.operation.starts_with("change.")
        || request.operation == "ci.rerun"
        || request.operation == crate::control::BREAK_GLASS;
    let mut owner = None;
    if let (Some(n), true) = (number, scoped) {
        let (status, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
        if status == 200 {
            owner = v["change"]["session"].as_str().map(str::to_string);
        }
    }
    let labels = guarded(
        &request.operation,
        text("session").as_deref(),
        owner.as_deref(),
    );
    allows(env, s, &labels, text("ticket").as_deref()).await
}

/// The `Instance` Durable Object's side: `/tickets/start` and `/tickets/list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Tickets = storage.get(TICKETS).await?.unwrap_or_default();
    match path {
        "/tickets/start" => {
            let label = body["label"].as_str().unwrap_or_default();
            let declared = serde_json::from_value(body["declared"].clone()).unwrap_or_default();
            match all.start_as(
                label,
                body["replace"].as_bool() == Some(true),
                now(),
                declared,
            ) {
                Ok(mut issued) => {
                    let parent = body["parent"].as_str().map(str::to_string);
                    all.parented(label, parent.clone());
                    issued.parent = parent;
                    storage.put(TICKETS, &all).await?;
                    Response::from_json(&issued)
                }
                Err(why) => Response::error(why, 409),
            }
        }
        "/tickets/list" => Response::from_json(&all),
        "/tickets/ended" => {
            let label = body["label"].as_str().unwrap_or_default();
            let ended = all.record_end(label, body["clean"].as_bool() == Some(true), now());
            storage.put(TICKETS, &all).await?;
            Response::from_json(&json!({ "archiver": ended.and_then(|e| e.archiver) }))
        }
        "/tickets/archived" => {
            let text = |k: &str| body[k].as_str().unwrap_or_default();
            match all.record_archive(
                text("label"),
                text("by"),
                text("outcome"),
                text("why"),
                now(),
            ) {
                Ok(a) => {
                    storage.put(TICKETS, &all).await?;
                    Response::from_json(&a)
                }
                Err(why) => Response::error(why, 409),
            }
        }
        "/tickets/lapse" => {
            let at = now();
            let lapsed: Vec<&str> = body["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|l| all.lapse(l, at).is_some())
                .collect();
            storage.put(TICKETS, &all).await?;
            Response::from_json(&json!({ "lapsed": lapsed }))
        }
        "/tickets/end" => {
            let ended = all.end(body["label"].as_str().unwrap_or_default(), now());
            storage.put(TICKETS, &all).await?;
            Response::from_json(&json!({ "ended": ended }))
        }
        _ => Response::error("not found", 404),
    }
}
