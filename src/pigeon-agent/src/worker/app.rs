//! The app's reads (its approvals list, the tracker, Auto's state) and Auto's start and end,
//! which the device door's tools and the proof submit call; see
//! `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its R10b amendment.

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result};

use super::approve::live;
use super::platform::Settings;
use super::to_change;
use crate::core::app::{approvals, tracker};
use crate::core::approval::windows;
use crate::core::change::Change;

/// The relying party the app's passkey answers for; its client data's origin is this host.
pub(super) fn relying_party(env: &Env) -> String {
    env.var("PASSKEY_RP_ID")
        .map_or_else(|_| "platform.pigeon.dev".into(), |v| v.to_string())
}

/// The waiting change approvals, as the app lists them.
///
/// # Errors
///
/// When a binding fails.
pub async fn list(env: &Env, s: &Settings) -> Result<Value> {
    Ok(approvals::list(&live(env, s).await?))
}

/// `approval.list`: the open fast lanes and every approval waiting; see the ADR's 2026-11-18 amendment §4.
///
/// # Errors
///
/// When a binding fails.
pub async fn approvals(env: &Env, s: &Settings) -> Result<Value> {
    let waiting = live(env, s).await?;
    let (_, lanes) = super::mandates::to_instance(env, s, "/lanes/list", &json!({})).await?;
    let open: Vec<Value> = lanes["lanes"]
        .as_array()
        .map(|a| a.iter().filter(|l| l["state"] == "open").cloned().collect())
        .unwrap_or_default();
    Ok(json!({ "lanes": open, "waiting": waiting }))
}

/// The Auto window running or paused, for the app's home.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_state(env: &Env, s: &Settings) -> Result<Value> {
    let (_, v) = super::mandates::to_instance(env, s, "/windows/current", &json!({})).await?;
    Ok(v["window"].clone())
}

/// `POST /v0/app/auto` with `{passkey}`: starts standing Auto, proved by the owner's passkey for
/// `window:start`; an older app's `hours` is ignored.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_start(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let form = approvals::proof_form(body);
    if let Err(why) =
        super::passkeys::prove(env, s, &form, windows::START, &relying_party(env)).await?
    {
        return Response::error(format!("refused: {why}"), 403);
    }
    let owner = super::account::approver(env, s).await?;
    let (status, v) =
        super::mandates::to_instance(env, s, "/windows/start", &json!({ "by": owner })).await?;
    if status != 200 {
        return Response::error(v.as_str().unwrap_or("Auto didn't start").to_string(), 409);
    }
    let said = format!(
        "Auto window {} started by {owner}, standing until the owner pauses it",
        v["window"]["id"].as_u64().unwrap_or_default()
    );
    let _ = super::events::append(env, s, "auto.started", None, None, &said, false).await;
    Response::from_json(&v)
}

/// `DELETE /v0/app/auto` with `{passkey}`: pauses Auto by ending its window, proved by the owner's
/// passkey for `window:end`.
///
/// # Errors
///
/// When a binding fails.
pub async fn auto_end(env: &Env, s: &Settings, body: &Value) -> Result<Response> {
    let form = approvals::proof_form(body);
    if let Err(why) =
        super::passkeys::prove(env, s, &form, windows::END, &relying_party(env)).await?
    {
        return Response::error(format!("refused: {why}"), 403);
    }
    let (status, v) = super::mandates::to_instance(
        env,
        s,
        "/windows/end",
        &json!({ "why": "the owner ended it" }),
    )
    .await?;
    if status != 200 {
        return Response::error(
            v.as_str()
                .unwrap_or("no Auto window is running")
                .to_string(),
            409,
        );
    }
    let said = format!(
        "Auto window {} ended by the owner",
        v["window"]["id"].as_u64().unwrap_or_default()
    );
    let _ = super::events::append(env, s, "auto.ended", None, None, &said, false).await;
    Response::from_json(&v)
}

/// How long a merged change stays on the tracker, in seconds.
const MERGED_KEPT: u64 = 3600;

/// `GET /v0/app/tracker`: each session and its changes on the rail.
///
/// # Errors
///
/// When a binding fails.
pub async fn tracker(env: &Env, s: &Settings) -> Result<Value> {
    let now = super::platform::now();
    let (_, t) = super::mandates::to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(t).unwrap_or_default();
    let rows =
        super::platform::d1_rows(env, &crate::control::list(&s.instance, &s.repo, None)).await?;
    let (merged, open): (Vec<Value>, Vec<Value>) = rows
        .into_iter()
        .filter(|r| match r["state"].as_str() {
            Some("closed") => false,
            Some("merged") => r["updated"]
                .as_u64()
                .is_some_and(|u| now.saturating_sub(u) <= MERGED_KEPT),
            _ => true,
        })
        .partition(|r| r["state"].as_str() == Some("merged"));
    let mut changes = Vec::new();
    for row in super::stages::list(env, s, open).await? {
        if let (Some(number), Ok(stage)) = (
            row["number"].as_u64().and_then(|n| u32::try_from(n).ok()),
            serde_json::from_value(row["stage"].clone()),
        ) {
            changes.push(tracker::ChangeIn {
                number,
                title: row["title"].as_str().unwrap_or_default().to_string(),
                session: row["session"].as_str().map(str::to_string),
                stage,
            });
        }
    }
    let queue = super::stages::queue(env, s).await;
    for row in merged {
        let Some(number) = row["number"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        let (_, view) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
        let Ok(change) = serde_json::from_value::<Change>(view["change"].clone()) else {
            continue;
        };
        let stage = crate::core::change::stage::Stage {
            since: row["updated"].as_u64(),
            ..super::stages::stage(env, s, &change, &queue).await
        };
        changes.push(tracker::ChangeIn {
            number,
            title: row["title"].as_str().unwrap_or_default().to_string(),
            session: row["session"].as_str().map(str::to_string),
            stage,
        });
    }
    let mut labels: Vec<String> = tickets
        .0
        .keys()
        .filter(|l| tickets.live(l, now).is_some())
        .cloned()
        .collect();
    labels.extend(changes.iter().filter_map(|c| c.session.clone()));
    labels.sort_unstable();
    labels.dedup();
    let mut sessions = Vec::new();
    for label in labels {
        let (_, cursor) =
            super::mandates::to_instance(env, s, "/events/cursor", &json!({ "session": label }))
                .await?;
        sessions.push(tracker::SessionIn {
            declared: tickets.live(&label, now).map(|i| i.declared),
            class: tickets.class(&label),
            polled: cursor["polled"].as_u64(),
            listening: cursor["listening"].as_bool() == Some(true),
            label,
        });
    }
    let briefs = super::briefs::all(env, s).await;
    Ok(serde_json::to_value(tracker::build(
        &sessions, &changes, &briefs, now,
    ))?)
}
