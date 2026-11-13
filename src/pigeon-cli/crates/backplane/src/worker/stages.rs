//! A change's stage with what the pure derivation needs gathered, and a session's position; see
//! `.claude/decisions/2026-11-13-development-session-stages-subscriptions-breaker.md` §1.

use serde_json::{Map, Value, json};
use worker::{Env, Method, Result};

use super::mandates::{to_instance, window_approves};
use super::platform::{self, Settings, json_request};
use crate::changes::{Change, State};
use crate::core::change::stage::{self, Context, Deploy, Stage};
use crate::events::PAGE;
use crate::merges::MergeQueue;

/// The most pages of a change's events read for `since`.
const PAGES: usize = 10;

/// The deploy carrying `merged`; unknown when `MainRun` can't say.
async fn carrying(env: &Env, s: &Settings, merged: &str) -> Deploy {
    let read = async {
        let body = json!({ "sha": merged });
        let req = json_request("https://main/carrying", Method::Post, Some(&body), None)?;
        let mut r = platform::call(env, &super::main_stub(env, &s.repo)?, req).await?;
        if r.status_code() != 200 {
            return Ok(Deploy::Unknown);
        }
        Ok::<_, worker::Error>(r.json().await.unwrap_or_default())
    };
    read.await.unwrap_or_default()
}

/// The window or mandate that would allow `c`'s next authorization or approval.
async fn covered(env: &Env, s: &Settings, c: &Change) -> Option<String> {
    let operation = match c.state {
        State::Proposed => "change.authorize",
        State::Checked => "change.approve",
        _ => return None,
    };
    if let Some(m) = c.mandate {
        return Some(format!("mandate:{m}"));
    }
    window_approves(env, s, c, operation)
        .await
        .ok()
        .flatten()
        .map(|id| format!("window:{id}"))
}

/// When change `number` last moved: its last event that isn't a job's.
async fn since(env: &Env, s: &Settings, number: u32) -> Option<u64> {
    let mut after = 0;
    let mut last = None;
    for _ in 0..PAGES {
        let body = json!({ "after": after, "changes": [number] });
        let (_, v) = to_instance(env, s, "/events/since", &body).await.ok()?;
        let events = v["events"].as_array()?;
        for e in events {
            after = e["id"].as_u64().unwrap_or(after);
            if !e["kind"].as_str().unwrap_or_default().starts_with("job.") {
                last = e["at"].as_u64().or(last);
            }
        }
        if events.len() < PAGE {
            break;
        }
    }
    last
}

/// Change `c`'s stage, with its place in `queue` and its `since`.
pub async fn of(env: &Env, s: &Settings, c: &Change, queue: &MergeQueue) -> Stage {
    Stage {
        since: since(env, s, c.number).await,
        ..stage(env, s, c, queue).await
    }
}

/// Change `c`'s stage, with its place in `queue`, without `since`: an event carries its own time.
pub async fn stage(env: &Env, s: &Settings, c: &Change, queue: &MergeQueue) -> Stage {
    let deploy = match (&c.state, &c.merged_as) {
        (State::Merged, Some(sha)) => carrying(env, s, sha).await,
        _ => Deploy::Unknown,
    };
    let cx = Context {
        place: stage::place(queue, c.number),
        deploy,
        covered: covered(env, s, c).await,
    };
    stage::of(c, &cx)
}

/// `change.list`'s rows with their stages: an open change's in full, a merged or closed one's
/// from its row, `since` its last update.
///
/// # Errors
///
/// When a binding fails.
pub async fn list(env: &Env, s: &Settings, mut rows: Vec<Value>) -> Result<Vec<Value>> {
    let merges = queue(env, s).await;
    for row in &mut rows {
        let state = row["state"].as_str().unwrap_or_default().to_string();
        let number = row["number"].as_u64().and_then(|n| u32::try_from(n).ok());
        let st = match (state.as_str(), number) {
            ("merged" | "closed", _) | (_, None) => Stage {
                stage: state.clone(),
                needs: stage::Needs::None,
                waits_on: None,
                next: "nothing".into(),
                since: row["updated"].as_u64(),
            },
            (_, Some(number)) => {
                let (_, view) =
                    super::to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
                match serde_json::from_value::<Change>(view["change"].clone()) {
                    Ok(c) => Stage {
                        since: row["updated"].as_u64(),
                        ..stage(env, s, &c, &merges).await
                    },
                    Err(_) => continue,
                }
            }
        };
        row["stage"] = serde_json::to_value(st)?;
    }
    Ok(rows)
}

/// The merge queue; empty when it can't be read.
pub async fn queue(env: &Env, s: &Settings) -> MergeQueue {
    to_instance(env, s, "/merges/list", &json!({}))
        .await
        .ok()
        .and_then(|(_, q)| serde_json::from_value(q).ok())
        .unwrap_or_default()
}

/// `session.status`: each open change of session `session` with its stage, oldest first, and
/// its tasks.
///
/// # Errors
///
/// When a binding fails.
pub async fn session(env: &Env, s: &Settings, args: &Map<String, Value>) -> Result<(u16, Value)> {
    let Some(label) = args
        .get("session")
        .and_then(Value::as_str)
        .filter(|l| !l.trim().is_empty())
    else {
        return Ok((400, json!({ "error": "name the session: its label" })));
    };
    let rows = platform::d1_rows(env, &crate::control::open(&s.instance, &s.repo)).await?;
    let mut numbers: Vec<u32> = crate::control::open_rows(&rows)
        .into_iter()
        .filter(|o| o.session.as_deref() == Some(label))
        .map(|o| o.number)
        .collect();
    numbers.sort_unstable();
    let q = queue(env, s).await;
    let mut changes = Vec::new();
    for n in numbers {
        let (_, v) = super::to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
        let Ok(c) = serde_json::from_value::<Change>(v["change"].clone()) else {
            continue;
        };
        let st = of(env, s, &c, &q).await;
        changes.push(json!({ "number": n, "title": c.title, "stage": st }));
    }
    let (_, tasks) = to_instance(env, s, "/tasks/list", &json!({ "session": label })).await?;
    let mine: Vec<&Value> = tasks["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["session"].as_str() == Some(label) || t["claimed_by"].as_str() == Some(label))
        .collect();
    let (_, cursor) = to_instance(env, s, "/events/cursor", &json!({ "session": label })).await?;
    Ok((
        200,
        json!({ "session": label, "changes": changes, "tasks": mine, "events": cursor }),
    ))
}
