//! The learner's routes on a standalone: progress, forms, attempts and sync, each an `Instance`
//! operation that checks the caller's `learner` capability and keeps every object sealed in the
//! `LEARNER` bucket; see `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`
//! §5 and its 2026-11-15 amendment.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use worker::{Env, Method, Response, Result, State};

use super::devices::DEVICES;
use super::elicit;
use super::platform::{self, Settings};
use crate::core::learner::{
    self, Attempt, Form, INDEX, Index, PREFIX, Progress, Sync, attempt_key, form_key, progress_key,
};
use crate::core::member::{self, Members};
use crate::devices::{Device, Devices};

/// The bucket binding.
const BUCKET: &str = "LEARNER";

/// The routes' prefix.
pub const API: &str = "/v0/app/learner/";

fn fail(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!({ "error": why, "code": code }))?.with_status(code))
}

/// `/v0/app/learner/*` for `device`: the route's `Instance` operation.
///
/// # Errors
///
/// When a binding fails.
pub async fn route(
    env: &Env,
    s: &Settings,
    device: &Device,
    method: &Method,
    path: &str,
    body: &Value,
) -> Result<Response> {
    let rest = path.trim_start_matches(API);
    let (op, arg) = match (method, rest) {
        (Method::Get, "progress") => ("/learner/progress", json!(null)),
        (Method::Get, "attempts") => ("/learner/attempts", json!(null)),
        (Method::Post, "sync") => ("/learner/sync", body.clone()),
        (Method::Post, "forms") => ("/learner/sync", json!({ "forms": [body] })),
        (Method::Post, "attempts") => ("/learner/sync", json!({ "attempts": [body] })),
        (Method::Get, r) if r.starts_with("forms/") => match r[6..].parse::<u32>() {
            Ok(n) => ("/learner/form", json!({ "number": n })),
            Err(_) => return Response::error("not a form number", 400),
        },
        _ => return Response::error("not found", 404),
    };
    let v = elicit::inbox(env, s, op, &json!({ "device": device.id, "body": arg })).await?;
    match v["error"].as_str() {
        Some(why) => {
            let code = v["code"].as_u64().and_then(|c| u16::try_from(c).ok());
            Response::error(why, code.unwrap_or(500))
        }
        None => Response::from_json(&v),
    }
}

// The `Instance` side.

fn features(env: &Env) -> bool {
    env.var("FEATURES")
        .is_ok_and(|v| v.to_string().split(',').any(|f| f.trim() == "learner"))
}

async fn allowed(state: &State, body: &Value) -> Result<bool> {
    let storage = state.storage();
    let devices: Devices = storage.get(DEVICES).await?.unwrap_or_default();
    let members: Members = storage.get(member::STORE).await?.unwrap_or_default();
    let id = body["device"].as_str().unwrap_or_default();
    let Some(d) = devices.devices.iter().find(|d| d.id == id) else {
        return Ok(false);
    };
    let Ok(roles) = super::standalone::roles() else {
        return Ok(false);
    };
    Ok(members
        .active(&d.account)
        .is_some_and(|m| roles.capabilities(m.role).iter().any(|c| c == "learner")))
}

fn key(env: &Env) -> Result<[u8; 32]> {
    let k = platform::secret(env, "STATE_KEY")?;
    learner::sealing_key(&k).map_err(worker::Error::RustError)
}

async fn put(env: &Env, key: &[u8; 32], name: String, value: &impl serde::Serialize) -> Result<()> {
    let hex = platform::random_hex(12)?;
    let mut nonce = [0u8; 12];
    for (i, b) in nonce.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2).unwrap_or("00"), 16).unwrap_or(0);
    }
    let plain = serde_json::to_vec(value).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let sealed = learner::seal(key, nonce, &plain).map_err(worker::Error::RustError)?;
    env.bucket(BUCKET)?.put(name, sealed).execute().await?;
    Ok(())
}

async fn get<T: serde::de::DeserializeOwned>(
    env: &Env,
    key: &[u8; 32],
    name: String,
) -> Result<Option<T>> {
    let Some(object) = env.bucket(BUCKET)?.get(name).execute().await? else {
        return Ok(None);
    };
    let Some(body) = object.body() else {
        return Ok(None);
    };
    let plain = learner::open(key, &body.bytes().await?).map_err(worker::Error::RustError)?;
    serde_json::from_slice(&plain)
        .map(Some)
        .map_err(|e| worker::Error::RustError(e.to_string()))
}

async fn names(env: &Env, prefix: &str) -> Result<Vec<String>> {
    let bucket = env.bucket(BUCKET)?;
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut list = bucket.list().prefix(prefix);
        if let Some(c) = cursor.take() {
            list = list.cursor(c);
        }
        let page = list.execute().await?;
        out.extend(page.objects().iter().map(worker::Object::key));
        if !page.truncated() {
            return Ok(out);
        }
        cursor = page.cursor();
    }
}

async fn all<T: serde::de::DeserializeOwned>(
    env: &Env,
    key: &[u8; 32],
    prefix: &str,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for name in names(env, prefix).await? {
        if let Some(v) = get(env, key, name).await? {
            out.push(v);
        }
    }
    Ok(out)
}

/// The index, rebuilt from the bucket when the `Instance` has lost it.
async fn index(state: &State, env: &Env, key: &[u8; 32]) -> Result<Index> {
    let storage = state.storage();
    if let Some(ix) = storage.get::<Index>(INDEX).await? {
        return Ok(ix);
    }
    let forms: Vec<Form> = all(env, key, &format!("{PREFIX}forms/")).await?;
    let attempts: Vec<Attempt> = all(env, key, &format!("{PREFIX}attempts/")).await?;
    let ix = Index::of(&forms, &attempts);
    storage.put(INDEX, &ix).await?;
    put(env, key, progress_key(), &learner::progress_of(&attempts)).await?;
    Ok(ix)
}

/// `/learner/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage or the bucket fails.
pub async fn serve(state: &State, env: &Env, path: &str, body: Value) -> Result<Response> {
    if !features(env) {
        return fail("this instance has no learner feature", 404);
    }
    if !allowed(state, &body).await? {
        return fail("this account may not practise here", 403);
    }
    let key = key(env)?;
    let ix = index(state, env, &key).await?;
    match path {
        "/learner/progress" => {
            let progress: Progress = get(env, &key, progress_key()).await?.unwrap_or_default();
            Response::from_json(&json!({ "attempts": ix.attempts, "progress": progress }))
        }
        "/learner/attempts" => {
            let attempts: Vec<Attempt> = all(env, &key, &format!("{PREFIX}attempts/")).await?;
            Response::from_json(&json!({ "attempts": attempts }))
        }
        "/learner/form" => {
            let n = body["body"]["number"].as_u64().unwrap_or(u64::MAX);
            let n = u32::try_from(n).unwrap_or(u32::MAX);
            match get::<Form>(env, &key, form_key(n)).await? {
                Some(f) => Response::from_json(&f),
                None => fail("no such form", 404),
            }
        }
        "/learner/sync" => {
            let Ok(sync) = serde_json::from_value::<Sync>(body["body"].clone()) else {
                return fail("not a sync: {forms, attempts}", 400);
            };
            let mut held = BTreeMap::new();
            for a in &sync.attempts {
                let sent = sync.forms.iter().any(|f| f.number == a.form);
                if !sent
                    && !held.contains_key(&a.form)
                    && ix.forms.contains_key(&a.form)
                    && let Some(f) = get::<Form>(env, &key, form_key(a.form)).await?
                {
                    held.insert(a.form, f);
                }
            }
            let planned = learner::plan(&ix, sync, &held);
            for f in &planned.forms {
                put(env, &key, form_key(f.number), f).await?;
            }
            for a in &planned.attempts {
                put(env, &key, attempt_key(&a.id), a).await?;
            }
            if !planned.attempts.is_empty() {
                let mut progress: Progress =
                    get(env, &key, progress_key()).await?.unwrap_or_default();
                for a in &planned.attempts {
                    learner::tally(&mut progress, a);
                }
                put(env, &key, progress_key(), &progress).await?;
            }
            state.storage().put(INDEX, &planned.index).await?;
            Response::from_json(&learner::answer(&planned.index, &planned.refused))
        }
        _ => fail("not found", 404),
    }
}
