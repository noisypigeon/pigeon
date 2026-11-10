//! Passkeys in the control plane: the `Instance` keeps them; `/passkeys` registers one with the
//! authenticator code; approval pages take one as a proof; see
//! `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §4.

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::approve::{html_with, refused};
use super::elicit;
use super::platform::{self, Settings, now};
use crate::access::Identity;
use crate::approve as page;
use crate::passkeys::{self, Keyring};
use crate::push::{b64url, b64url_decode};
use crate::totp::Guard;

/// Where the `Instance` keeps the keyring.
const KEYRING: &str = "passkeys";

/// The script every proof page loads.
const SCRIPT: &str = include_str!("passkey.js");

/// The relying party ID for a page on `host`, from `PASSKEY_RP_ID`; see
/// `.claude/decisions/2026-11-10-runtime-platform-hostname-dashboard-observability.md` §2.
fn rp_for(env: &Env, host: &str) -> String {
    let configured = env.var("PASSKEY_RP_ID").ok().map(|v| v.to_string());
    passkeys::relying_party(host, configured.as_deref())
}

/// Whether `path` is one of the passkey pages.
#[must_use]
pub fn is_page(path: &str) -> bool {
    path == "/passkeys" || path == "/passkeys/challenge" || path == "/passkey.js"
}

// The `Instance` side.

/// `/passkeys/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let mut ring: Keyring = storage.get(KEYRING).await?.unwrap_or_default();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    // A page from before `rp` was sent asks for its own host.
    let rp = Some(s("rp"))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| s("host"));
    let bytes = |k: &str| b64url_decode(body[k].as_str().unwrap_or_default()).unwrap_or_default();
    let answer = match path {
        "/passkeys/list" => json!({
            "keys": ring.keys.iter().map(|k| json!({
                "id": k.id, "name": k.name, "created": k.created, "used": k.used,
            })).collect::<Vec<_>>(),
        }),
        "/passkeys/challenge" => {
            let random = hex(&platform::random_hex(32)?);
            let challenge = ring.mint(&s("purpose"), &random, now());
            let ids: Vec<&str> = ring.keys.iter().map(|k| k.id.as_str()).collect();
            json!({ "challenge": challenge, "ids": ids, "user": b64url(passkeys::USER) })
        }
        "/passkeys/register" => outcome(ring.register(
            &s("id"),
            &s("name"),
            &bytes("client"),
            &bytes("authenticator"),
            &bytes("spki"),
            &s("host"),
            &rp,
            now(),
        )),
        "/passkeys/prove" => {
            let mut guard: Guard = storage.get(elicit::GUARD).await?.unwrap_or_default();
            let t = now();
            let proved = if guard.locked(t) {
                Err("five failed proofs in the last hour; approvals by proof are locked for up to an hour".to_string())
            } else {
                ring.prove(
                    &s("id"),
                    &bytes("client"),
                    &bytes("authenticator"),
                    &bytes("signature"),
                    &s("purpose"),
                    &s("host"),
                    &rp,
                    t,
                )
            };
            if proved.is_err() && !guard.locked(t) {
                guard.failures.push(t);
                storage.put(elicit::GUARD, &guard).await?;
            }
            outcome(proved)
        }
        "/passkeys/remove" => {
            json!({ "status": if ring.remove(&s("id")) { "ok" } else { "gone" } })
        }
        _ => return Response::error("not found", 404),
    };
    storage.put(KEYRING, &ring).await?;
    Response::from_json(&answer)
}

fn outcome(r: std::result::Result<(), String>) -> Value {
    match r {
        Ok(()) => json!({ "status": "ok" }),
        Err(why) => json!({ "status": "refused", "why": why }),
    }
}

/// Bytes from lowercase hex; nothing for anything else.
fn hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .filter_map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok())
        .collect()
}

// The pages.

async fn to_instance(env: &Env, s: &Settings, path: &str, body: &Value) -> Result<Value> {
    elicit::inbox(env, s, path, body).await
}

/// Checks a passkey answer posted with `form` for `purpose` on `host`.
///
/// # Errors
///
/// When the binding fails.
pub async fn prove(
    env: &Env,
    s: &Settings,
    form: &str,
    purpose: &str,
    host: &str,
) -> Result<std::result::Result<(), String>> {
    let f = |k: &str| page::field(form, k).unwrap_or_default();
    let v = to_instance(
        env,
        s,
        "/passkeys/prove",
        &json!({
            "id": f("pk_id"), "client": f("pk_client"), "authenticator": f("pk_authenticator"),
            "signature": f("pk_signature"), "purpose": purpose, "host": host,
            "rp": rp_for(env, host),
        }),
    )
    .await?;
    Ok(if v["status"] == "ok" {
        Ok(())
    } else {
        Err(v["why"]
            .as_str()
            .unwrap_or("the passkey was refused")
            .to_string())
    })
}

/// Whether `form` carries a passkey answer rather than a code.
#[must_use]
pub fn answered(form: &str) -> bool {
    page::field(form, "pk_client").is_some_and(|c| !c.is_empty())
}

/// Serves `/passkeys`, `/passkeys/challenge` and `/passkey.js` for `who`.
///
/// # Errors
///
/// When a binding fails.
pub async fn page_serve(
    mut req: Request,
    env: &Env,
    s: &Settings,
    who: &Identity,
) -> Result<Response> {
    let path = req.path();
    if path == "/passkey.js" {
        let mut r = Response::ok(SCRIPT)?;
        r.headers_mut()
            .set("content-type", "text/javascript; charset=utf-8")?;
        return Ok(r);
    }
    if !who.human {
        return refused(403, "only a person signed in to Access manages passkeys");
    }
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    if req.method() != Method::Get {
        let origin = req.headers().get("origin")?;
        let site = req.headers().get("sec-fetch-site")?;
        if !page::same_origin(origin.as_deref(), site.as_deref(), &host) {
            return refused(403, "this is posted only from its own page");
        }
    }
    match (req.method(), path.as_str()) {
        (Method::Post, "/passkeys/challenge") => {
            let body: Value = req.json().await?;
            let purpose = body["purpose"].as_str().unwrap_or_default();
            if purpose != passkeys::REGISTER && !purpose.starts_with("grant:") {
                return refused(400, "no such purpose");
            }
            let mut v = to_instance(
                env,
                s,
                "/passkeys/challenge",
                &json!({ "purpose": purpose }),
            )
            .await?;
            v["rp"] = json!(rp_for(env, &host));
            Response::from_json(&v)
        }
        (Method::Get, "/passkeys") => list(env, s, None).await,
        (Method::Post, "/passkeys") => {
            let form = req.text().await?;
            let code = page::field(&form, "code").unwrap_or_default();
            if let Err(why) = elicit::check_code(env, s, code.trim()).await? {
                return refused(403, &why);
            }
            let f = |k: &str| page::field(&form, k).unwrap_or_default();
            let said = if let Some(id) = page::field(&form, "remove") {
                to_instance(env, s, "/passkeys/remove", &json!({ "id": id })).await?;
                "Removed.".to_string()
            } else {
                let v = to_instance(
                    env,
                    s,
                    "/passkeys/register",
                    &json!({
                        "id": f("pk_id"), "name": f("name"), "client": f("pk_client"),
                        "authenticator": f("pk_authenticator"), "spki": f("pk_spki"), "host": host,
                        "rp": rp_for(env, &host),
                    }),
                )
                .await?;
                if v["status"] == "ok" {
                    "Registered. It now approves wherever the code does on these pages.".to_string()
                } else {
                    format!("Refused: {}", v["why"].as_str().unwrap_or("unknown"))
                }
            };
            list(env, s, Some(&said)).await
        }
        _ => refused(404, "not found"),
    }
}

async fn list(env: &Env, s: &Settings, said: Option<&str>) -> Result<Response> {
    let v = to_instance(env, s, "/passkeys/list", &json!({})).await?;
    let keys: Vec<(String, String, u64, Option<u64>)> = v["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            (
                k["id"].as_str().unwrap_or_default().to_string(),
                k["name"].as_str().unwrap_or_default().to_string(),
                k["created"].as_u64().unwrap_or_default(),
                k["used"].as_u64(),
            )
        })
        .collect();
    html_with(200, &page::passkeys_page(&keys, said), page::PROOF_CSP)
}
