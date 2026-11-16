//! The app's instances on the platform: `GET /v0/app/instances` lists the platform and the
//! personal instances the caller administers, from `instances.private`; and
//! `POST /v0/app/instances/standup` stands one up on a Runner with the owner's pasted token,
//! proved by their passkey; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md` §2 and its
//! 2026-11-15 amendments.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use worker::{Env, Method, Response, Result, State};

use super::devices::DEVICES;
use super::elicit;
use super::passkeys::KEYRING;
use super::platform::{self, Settings, job_stub, json_request, now};
use crate::control::{self, JobSpec, Outcome, Report};
use crate::core::app::devices::{Device, Devices};
use crate::core::approval::passkeys::Keyring;
use crate::core::member::handover::{HandedDevice, HandedPasskey, Handover};
use crate::core::member::listing::{self, Listing, STACK_INSTANCE};
use crate::core::member::{self, Members, Role};

fn fail(why: &str, code: u16) -> Result<Response> {
    Ok(Response::from_json(&json!({ "error": why, "code": code }))?.with_status(code))
}

fn answer(v: &Value) -> Result<Response> {
    match v["error"].as_str() {
        Some(why) => {
            let code = v["code"].as_u64().and_then(|c| u16::try_from(c).ok());
            Response::error(why, code.unwrap_or(500))
        }
        None => Response::from_json(v),
    }
}

// The `Instance` side.

/// `/instances/*` on the platform's `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let listing: Listing = storage.get(listing::STORE).await?.unwrap_or_default();
    if path == "/instances/finished" {
        let job = body["job"].as_str().unwrap_or_default();
        let outcome = match body["base_url"].as_str() {
            Some(b) => Ok(b.to_string()),
            None => Err(body["why"].as_str().unwrap_or("the job failed").to_string()),
        };
        storage
            .put(listing::STORE, &listing.finished(job, outcome))
            .await?;
        return Response::from_json(&json!({ "status": "ok" }));
    }
    let devices: Devices = storage.get(DEVICES).await?.unwrap_or_default();
    let id = body["device"].as_str().unwrap_or_default();
    let Some(caller) = devices.devices.iter().find(|d| d.id == id).cloned() else {
        return fail("no such device", 404);
    };
    match path {
        "/instances/list" => Response::from_json(&json!({
            "instances": listing.shown(&caller.account),
            "running": listing.running(),
        })),
        "/instances/begin" => {
            let members: Members = storage.get(member::STORE).await?.unwrap_or_default();
            let admin = members
                .active(&caller.account)
                .is_some_and(|m| m.role == Role::Administrator);
            if !admin {
                return fail("only an administrator stands up an instance", 403);
            }
            let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
            let begun = listing.begin(
                &s("id"),
                &s("name"),
                &s("accent"),
                &caller.account,
                &s("job"),
                now(),
            );
            let (listing, entry) = match begun {
                Ok(b) => b,
                Err(why) => return fail(&why, 409),
            };
            // Its label under the platform's zone; see
            // .claude/decisions/2026-11-15-runtime-standalone-hostnames-platform-zone.md §5.
            let (listing, entry) = match body["zone"].as_str().filter(|z| !z.is_empty()) {
                None => (listing, entry),
                Some(zone) => match listing.named(&entry.id, &s("label"), zone) {
                    Ok(l) => {
                        let e = l.entries.iter().find(|e| e.id == entry.id).cloned();
                        (l, e.unwrap_or(entry))
                    }
                    Err(why) => return fail(&why, 409),
                },
            };
            let ring: Keyring = storage.get(KEYRING).await?.unwrap_or_default();
            let handover = Handover {
                owner: caller.account.clone(),
                passkeys: ring
                    .keys
                    .iter()
                    .map(|k| HandedPasskey {
                        id: k.id.clone(),
                        key: k.key.clone(),
                        name: k.name.clone(),
                        synced: k.synced,
                    })
                    .collect(),
                devices: devices
                    .devices
                    .iter()
                    .filter(|d| d.account == caller.account)
                    .map(|d| HandedDevice {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        key: d.key.clone(),
                    })
                    .collect(),
            };
            storage.put(listing::STORE, &listing).await?;
            Response::from_json(&json!({ "entry": entry, "handover": handover }))
        }
        _ => fail("not found", 404),
    }
}

// The Worker side.

/// The stand-up job `id`'s result once it has finished: its report's `platform` value, or why it
/// failed.
async fn poll(env: &Env, id: &str) -> Option<std::result::Result<Value, String>> {
    let stub = job_stub(env, control::class_of(control::PLATFORM), id).ok()?;
    let mut r = stub
        .fetch_with_str(&format!("https://checker/status/{id}"))
        .await
        .ok()?;
    if r.status_code() == 404 {
        return Some(Err("its job was lost; stand it up again".into()));
    }
    match r.json::<Report>().await {
        Ok(rep) if rep.outcome == Outcome::Running => None,
        Ok(rep) => Some(match (rep.outcome, rep.platform) {
            (Outcome::Passed, Some(p)) => Ok(p),
            _ => Err(rep
                .violations
                .first()
                .cloned()
                .unwrap_or_else(|| "the job failed".into())),
        }),
        Err(_) => None,
    }
}

/// `GET /v0/app/instances`: the platform, then each personal instance the caller administers;
/// a stand-up that finished is recorded first.
///
/// # Errors
///
/// When a binding fails.
pub async fn list(env: &Env, s: &Settings, device: &Device) -> Result<Response> {
    let mut v = elicit::inbox(env, s, "/instances/list", &json!({ "device": device.id })).await?;
    if v["error"].is_string() {
        return answer(&v);
    }
    let running: Vec<String> = serde_json::from_value(v["running"].clone()).unwrap_or_default();
    let mut changed = false;
    for job in running {
        if let Some(r) = poll(env, &job).await {
            let done = match r {
                Ok(p) => json!({ "job": job, "base_url": p["base_url"] }),
                Err(why) => json!({ "job": job, "why": why }),
            };
            elicit::inbox(env, s, "/instances/finished", &done).await?;
            changed = true;
        }
    }
    if changed {
        v = elicit::inbox(env, s, "/instances/list", &json!({ "device": device.id })).await?;
    }
    let mut out = vec![json!({
        "id": "platform", "name": "Platform", "mode": "platform", "shape": "full",
        "base_url": format!("https://{}", super::app::relying_party(env)),
        "accent": "", "features": [], "status": "ready", "current": true,
    })];
    out.extend(v["instances"].as_array().cloned().unwrap_or_default());
    Response::from_json(&json!({ "instances": out }))
}

/// The SHA-256 of `token`, hex, as the stand-up's passkey challenge is bound to it.
#[must_use]
pub fn token_sha(token: &str) -> String {
    crate::core::approval::grants::hex(&Sha256::digest(token.as_bytes()))
}

/// `POST /v0/app/instances/standup` with `{name, accent, token, passkey}`: the passkey proved
/// for this token, the entry begun, and the job started on a Runner with the token and the
/// owner's public keys alone.
///
/// # Errors
///
/// When a binding fails.
pub async fn standup(env: &Env, s: &Settings, device: &Device, body: &Value) -> Result<Response> {
    let token = body["token"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    if token.len() < 20 {
        return Response::error("paste the account token", 400);
    }
    let form = crate::core::app::approvals::proof_form(body);
    let purpose = listing::standup_purpose(&token_sha(&token));
    let rp = super::app::relying_party(env);
    if let Err(why) = super::passkeys::prove(env, s, &form, &purpose, &rp).await? {
        return Response::error(format!("refused: {why}"), 403);
    }
    let job = format!("standup-{}", platform::random_hex(8)?);
    let begin = json!({
        "device": device.id, "id": platform::random_hex(8)?, "job": job,
        "name": body["name"], "accent": body["accent"].as_str().unwrap_or("#3a7d5c"),
        "label": body["label"], "zone": env.var("INSTANCE_ZONE").map(|v| v.to_string()).unwrap_or_default(),
    });
    let v = elicit::inbox(env, s, "/instances/begin", &begin).await?;
    if v["error"].is_string() {
        return answer(&v);
    }
    let handover = v["handover"].to_string();
    let base = v["entry"]["host"].as_str().map(|h| format!("https://{h}"));
    if let Err(why) = start(env, &job, &token, &handover, base.as_deref()).await {
        let failed = json!({ "job": job, "why": format!("the job didn't start: {why}") });
        elicit::inbox(env, s, "/instances/finished", &failed).await?;
        return Response::error("the stand-up didn't start; try again", 502);
    }
    Response::from_json(&json!({ "instance": v["entry"], "status": "standing-up" }))
}

async fn deployed(env: &Env, s: &Settings) -> std::result::Result<String, String> {
    let ns = env.durable_object("MAIN_RUN").map_err(|e| e.to_string())?;
    let stub = ns
        .id_from_name(&format!("{}#main", s.repo))
        .and_then(|i| i.get_stub())
        .map_err(|e| e.to_string())?;
    let req =
        json_request("https://main/status", Method::Get, None, None).map_err(|e| e.to_string())?;
    let mut r = platform::call(env, &stub, req)
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    v["pipeline"]["deployed"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "main has no deploy yet to run the job from".into())
}

async fn start(
    env: &Env,
    job: &str,
    token: &str,
    handover: &str,
    base: Option<&str>,
) -> std::result::Result<(), String> {
    let s = Settings::from(env).map_err(|e| e.to_string())?;
    let sha = deployed(env, &s).await?;
    let (remote, git) = platform::credentials(env, false, crate::wire::artifacts_wire::TOKEN_TTL)
        .await
        .map_err(|e| e.to_string())?;
    let spec = JobSpec {
        id: job.to_string(),
        remote,
        token: git,
        base: "main".into(),
        branch: "main".into(),
        sha,
        number: 0,
        title: "a personal instance's stand-up".into(),
        plan: String::new(),
        scope: Vec::new(),
        job: control::PLATFORM.into(),
        argv: vec!["standup".into(), STACK_INSTANCE.into()],
        approver: None,
        since: None,
        env: BTreeMap::from([
            ("CLOUDFLARE_API_TOKEN".to_string(), token.to_string()),
            ("HANDOVER".to_string(), handover.to_string()),
            ("BASE_URL".to_string(), base.unwrap_or_default().to_string()),
        ]),
    };
    let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
    let req = json_request("https://checker/start", Method::Post, Some(&body), None)
        .map_err(|e| e.to_string())?;
    let mut r = job_stub(env, control::class_of(control::PLATFORM), job)
        .map_err(|e| e.to_string())?
        .fetch_with_request(req)
        .await
        .map_err(|e| e.to_string())?;
    if (200..300).contains(&r.status_code()) {
        Ok(())
    } else {
        Err(r.text().await.unwrap_or_default())
    }
}
