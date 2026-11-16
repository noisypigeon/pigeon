//! Owner items in the control plane: `owner.ask` and `owner.withdraw`, the `Instance` object's
//! store under `/owner-items/`, and the owner's answer from the app; see
//! `.claude/decisions/2026-11-15-security-standing-auto-owner-flags.md` §4.

use serde_json::{Map, Value, json};
use worker::{Env, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{Settings, now};
use crate::core::owner::items::{Ask, Item, Items, Kind};

/// The `Instance` storage key the items live under.
const ITEMS: &str = "owner_items";

fn text(args: &Map<String, Value>, k: &str) -> String {
    match args.get(k) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn number(args: &Map<String, Value>, k: &str) -> Option<u32> {
    text(args, k).trim_start_matches('!').parse().ok()
}

/// Runs `owner.ask` or `owner.withdraw` with `args`: its status and answer.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(
    env: &Env,
    s: &Settings,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(u16, Value)> {
    let session = text(args, "session");
    match operation {
        "owner.ask" => {
            let kind = match Kind::parse(&text(args, "kind")) {
                Ok(k) => k,
                Err(why) => return Ok((400, json!(why))),
            };
            let options = text(args, "options")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let body = json!({
                "kind": kind, "session": session, "change": number(args, "change"),
                "title": text(args, "title"), "detail": text(args, "detail"), "options": options,
            });
            let (status, v) = to_instance(env, s, "/owner-items/ask", &body).await?;
            let Ok(item) = serde_json::from_value::<Item>(v.clone()) else {
                return Ok((if status == 200 { 500 } else { status }, v));
            };
            let said = format!("{} #{}: {}", kind.name(), item.id, item.title);
            super::events::append(
                env,
                s,
                "owner.asked",
                item.change,
                Some(session),
                &said,
                true,
            )
            .await?;
            super::push::notify(env, s, kind.headline(), &item.title, "/approvals").await;
            Ok((
                200,
                json!({ "item": item, "waits": "`owner.answered` wakes you" }),
            ))
        }
        "owner.withdraw" => {
            let body = json!({ "id": number(args, "id"), "session": session });
            let (status, v) = to_instance(env, s, "/owner-items/withdraw", &body).await?;
            let Ok(item) = serde_json::from_value::<Item>(v.clone()) else {
                return Ok((if status == 200 { 500 } else { status }, v));
            };
            let said = format!(
                "{} #{} withdrawn: {}",
                item.kind.name(),
                item.id,
                item.title
            );
            super::events::append(
                env,
                s,
                "owner.withdrawn",
                item.change,
                Some(session),
                &said,
                false,
            )
            .await?;
            Ok((200, json!({ "item": item })))
        }
        other => Ok((404, json!(format!("`{other}` is not an owner operation")))),
    }
}

/// The open items, oldest first, for `GET /v0/app/home`.
///
/// # Errors
///
/// When a binding fails.
pub async fn open(env: &Env, s: &Settings) -> Result<Value> {
    let (_, v) = to_instance(env, s, "/owner-items/list", &json!({})).await?;
    let all: Items = serde_json::from_value(v).unwrap_or_default();
    Ok(json!(all.open()))
}

/// The digest's lines, one per open item.
///
/// # Errors
///
/// When a binding fails.
pub async fn lines(env: &Env, s: &Settings) -> Result<Vec<String>> {
    let (_, v) = to_instance(env, s, "/owner-items/list", &json!({})).await?;
    let all: Items = serde_json::from_value(v).unwrap_or_default();
    Ok(all.lines(now()))
}

/// `POST /v0/app/items/<id>` with `{answer}`: the owner answers item `id` from their device,
/// waking its session with `owner.answered`.
///
/// # Errors
///
/// When a binding fails.
pub async fn answer(env: &Env, s: &Settings, id: &str, body: &Value) -> Result<Response> {
    let Ok(id) = id.parse::<u32>() else {
        return Response::error("no such item", 404);
    };
    let by = super::account::approver(env, s).await?;
    let req = json!({ "id": id, "answer": body["answer"], "by": by });
    let (status, v) = to_instance(env, s, "/owner-items/answer", &req).await?;
    let Ok(item) = serde_json::from_value::<Item>(v.clone()) else {
        let why = v
            .as_str()
            .unwrap_or("the item can't be answered")
            .to_string();
        return Response::error(why, if status == 200 { 500 } else { 409 });
    };
    let said = format!(
        "{} #{} answered by {by}: {}",
        item.kind.name(),
        item.id,
        item.answer.as_deref().unwrap_or_default()
    );
    super::events::append(
        env,
        s,
        "owner.answered",
        item.change,
        Some(item.session.clone()),
        &said,
        false,
    )
    .await?;
    Response::from_json(&json!({ "item": item }))
}

/// The `Instance` Durable Object's side: `/owner-items/ask`, `answer`, `withdraw` and `list`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let all: Items = storage.get(ITEMS).await?.unwrap_or_default();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let id = body["id"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_default();
    let t = now();
    let done = match path {
        "/owner-items/list" => return Response::from_json(&all),
        "/owner-items/ask" => {
            let ask = Ask {
                kind: serde_json::from_value(body["kind"].clone()).unwrap_or(Kind::Action),
                session: s("session"),
                change: body["change"].as_u64().and_then(|n| u32::try_from(n).ok()),
                title: s("title"),
                detail: s("detail"),
                options: serde_json::from_value(body["options"].clone()).unwrap_or_default(),
            };
            all.ask(&ask, t)
        }
        "/owner-items/answer" => all.answer(id, &s("answer"), &s("by"), t),
        "/owner-items/withdraw" => all.withdraw(id, &s("session"), t),
        _ => return Response::error("not found", 404),
    };
    match done {
        Ok((next, item)) => {
            storage.put(ITEMS, &next).await?;
            Response::from_json(&item)
        }
        Err(why) => Response::error(why, 409),
    }
}
