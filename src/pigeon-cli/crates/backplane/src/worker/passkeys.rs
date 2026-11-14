//! Passkeys in the control plane: the `Instance` keeps them; `/passkeys` adds and removes them;
//! every page takes the owner's proof through [`on_page`]; see
//! `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §4 and
//! `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §1 to §3.

use serde_json::{Value, json};
use sha2::Digest as _;
use worker::{Env, Method, Request, Response, Result, State};

use super::approve::{html_with, refused};
use super::elicit;
use super::platform::{self, Settings, now};
use crate::access::Identity;
use crate::approve as page;
use crate::passkeys::{self, Bound, Keyring};
use crate::push::{b64url, b64url_decode};
use crate::totp::Guard;

/// Where the `Instance` keeps the keyring.
const KEYRING: &str = "passkeys";

/// Where the `Instance` keeps until when a proven owner may add a passkey.
const ADDING: &str = "passkey-adding";

/// The fewest passkeys a removal leaves, until the cutover raises it to two.
const MIN_KEYS: usize = 1;

/// The script every proof page loads.
const SCRIPT: &str = include_str!("passkey.js");

/// The relying party ID for a page on `host`, from `PASSKEY_RP_ID`; see
/// `.claude/decisions/2026-11-10-runtime-platform-hostname-dashboard-observability.md` §2.
fn rp_for(env: &Env, host: &str) -> String {
    let configured = env.var("PASSKEY_RP_ID").ok().map(|v| v.to_string());
    passkeys::relying_party(host, configured.as_deref())
}

// The `Instance` side.

/// `/passkeys/*` on the `Instance` Durable Object.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: Value) -> Result<Response> {
    let storage = state.storage();
    let ring: Keyring = storage.get(KEYRING).await?.unwrap_or_default();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    // A page from before `rp` was sent asks for its own host.
    let rp = Some(s("rp"))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| s("host"));
    let bytes = |k: &str| b64url_decode(body[k].as_str().unwrap_or_default()).unwrap_or_default();
    let (ring, answer) = match path {
        "/passkeys/list" => {
            let adding: u64 = storage.get(ADDING).await?.unwrap_or_default();
            let listed = json!({
            "keys": ring.keys.iter().map(|k| json!({
                "id": k.id, "name": k.name, "created": k.created, "used": k.used,
                "synced": k.synced,
            })).collect::<Vec<_>>(),
            "adding": adding > now(),
            });
            (ring, listed)
        }
        "/passkeys/challenge" => {
            let random = hex(&platform::random_hex(32)?);
            let (ring, challenge) = match serde_json::from_value::<Bound>(body["bound"].clone()) {
                Ok(bound) => ring.mint_bound(&s("purpose"), &bound, &random, now()),
                Err(_) => ring.mint(&s("purpose"), &random, now()),
            };
            let ids: Vec<&str> = ring.keys.iter().map(|k| k.id.as_str()).collect();
            let answer =
                json!({ "challenge": challenge, "ids": ids, "user": b64url(passkeys::USER) });
            (ring, answer)
        }
        "/passkeys/allow" => {
            storage
                .put(ADDING, &(now() + passkeys::CHALLENGE_TTL))
                .await?;
            (ring, json!({ "status": "ok" }))
        }
        "/passkeys/register" => {
            if body["coded"].as_bool() != Some(true) {
                let adding: u64 = storage.get(ADDING).await?.unwrap_or_default();
                if adding <= now() {
                    return Response::from_json(&outcome(Err(
                        "prove with an existing passkey first".into(),
                    )));
                }
                storage.delete(ADDING).await?;
            }
            let (ring, registered) = ring.register(
                &s("id"),
                &s("name"),
                &bytes("client"),
                &bytes("authenticator"),
                &bytes("spki"),
                &s("host"),
                &rp,
                now(),
            );
            (ring, outcome(registered))
        }
        "/passkeys/prove" => proving(state, ring, &body, &rp).await?,
        "/passkeys/remove" if body["min"].is_u64() => {
            let min = usize::try_from(body["min"].as_u64().unwrap_or_default()).unwrap_or(1);
            let (ring, removed) = ring.remove_keeping(&s("id"), min);
            (ring, outcome(removed))
        }
        "/passkeys/remove" => {
            let (ring, removed) = ring.remove(&s("id"));
            (
                ring,
                json!({ "status": if removed { "ok" } else { "gone" } }),
            )
        }
        _ => return Response::error("not found", 404),
    };
    storage.put(KEYRING, &ring).await?;
    Response::from_json(&answer)
}

/// `/passkeys/prove`: a bound challenge's answer against `body.bound`, under its passkey's own
/// lock; any other, as before, under the shared guard.
async fn proving(state: &State, ring: Keyring, body: &Value, rp: &str) -> Result<(Keyring, Value)> {
    let storage = state.storage();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let bytes = |k: &str| b64url_decode(body[k].as_str().unwrap_or_default()).unwrap_or_default();
    if ring.bound(&bytes("client")) {
        let Ok(bound) = serde_json::from_value::<Bound>(body["bound"].clone()) else {
            let why = "what this passkey signed for is gone; reload the page";
            return Ok((ring, json!({ "status": "refused", "why": why })));
        };
        let (ring, proved) = ring.prove_bound(
            &s("id"),
            &bytes("client"),
            &bytes("authenticator"),
            &bytes("signature"),
            &s("purpose"),
            &bound,
            &s("host"),
            rp,
            now(),
        );
        let answer = match proved {
            Ok(evidence) => json!({ "status": "ok", "evidence": evidence }),
            Err(why) => json!({ "status": "refused", "why": why }),
        };
        return Ok((ring, answer));
    }
    let mut guard: Guard = storage.get(elicit::GUARD).await?.unwrap_or_default();
    let t = now();
    let (ring, proved) = if guard.locked(t) {
        (ring, Err("five failed proofs in the last hour; approvals by proof are locked for up to an hour".to_string()))
    } else {
        ring.prove(
            &s("id"),
            &bytes("client"),
            &bytes("authenticator"),
            &bytes("signature"),
            &s("purpose"),
            &s("host"),
            rp,
            t,
        )
    };
    if proved.is_err() && !guard.locked(t) {
        guard.failures.push(t);
        storage.put(elicit::GUARD, &guard).await?;
    }
    Ok((ring, outcome(proved)))
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

/// What the grant behind `path`, a proof page's path and query, binds a proof to, and the purpose
/// its challenge must have, when the page fixes one; nothing for a page not yet bound.
async fn bound_for(env: &Env, s: &Settings, path: &str) -> Result<Option<(Option<String>, Bound)>> {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let token = page::field(query, "grant").unwrap_or_default();
    let purpose = Some(passkeys::grant_purpose(&token));
    if path == "/passkeys" {
        return Ok(Some((None, Bound::default())));
    }
    if let Some(n) = path
        .strip_prefix("/approve/")
        .and_then(|n| n.parse::<u32>().ok())
    {
        let body = json!({ "token": token });
        let (status, v) =
            super::to_change(env, &s.repo, n, Method::Post, "/peek", Some(&body)).await?;
        let Ok(grant) = serde_json::from_value::<crate::grants::Grant>(v["grant"].clone()) else {
            return Ok(None);
        };
        return Ok((status == 200).then(|| {
            let bound = Bound {
                operation: grant.operation,
                target: n.to_string(),
                head: grant.head.unwrap_or_default(),
            };
            (purpose, bound)
        }));
    }
    if let Some(id) = path.strip_prefix("/mandate/") {
        let v = to_instance(env, s, "/mandates/peek", &json!({ "token": token })).await?;
        let named = v["id"].as_u64().map(|n| n.to_string());
        if !v.is_object() || named.as_deref() != Some(id) {
            return Ok(None);
        }
        let shown = sha2::Sha256::digest(v.to_string().as_bytes());
        let bound = Bound {
            operation: "mandate.grant".into(),
            target: id.to_string(),
            head: crate::grants::hex(&shown),
        };
        return Ok(Some((purpose, bound)));
    }
    Ok(None)
}

/// Checks the passkey answer in `form`'s `<prefix>…` fields for `purpose` on `host`, bound to
/// what the page at `path` approves.
async fn prove_with(
    env: &Env,
    s: &Settings,
    form: &str,
    purpose: &str,
    host: &str,
    path: &str,
) -> Result<std::result::Result<(), String>> {
    let f = |k: &str| page::field(form, k).unwrap_or_default();
    let mut asked = json!({
        "id": f("pk_id"), "client": f("pk_client"), "authenticator": f("pk_authenticator"),
        "signature": f("pk_signature"), "purpose": purpose, "host": host,
        "rp": rp_for(env, host),
    });
    if let Some((_, bound)) = bound_for(env, s, path).await? {
        asked["bound"] = json!(bound);
    }
    let v = to_instance(env, s, "/passkeys/prove", &asked).await?;
    Ok(if v["status"] == "ok" {
        Ok(())
    } else {
        Err(v["why"]
            .as_str()
            .unwrap_or("the passkey was refused")
            .to_string())
    })
}

/// The owner's proof on the page at `path`, posted as `form`: a passkey answer for `purpose`,
/// or, until the cutover, the code. Every page asks through this; see
/// `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §1.
///
/// # Errors
///
/// When a binding fails.
pub async fn on_page(
    env: &Env,
    s: &Settings,
    form: &str,
    purpose: &str,
    host: &str,
    path: &str,
) -> Result<std::result::Result<(), String>> {
    if answered(form) {
        prove_with(env, s, form, purpose, host, path).await
    } else {
        let code = page::field(form, "code").unwrap_or_default();
        elicit::check_code(env, s, code.trim()).await
    }
}

/// Whether the owner has registered a passkey.
///
/// # Errors
///
/// When the binding fails.
pub(super) async fn registered(env: &Env, s: &Settings) -> Result<bool> {
    let v = to_instance(env, s, "/passkeys/list", &json!({})).await?;
    Ok(v["keys"].as_array().is_some_and(|k| !k.is_empty()))
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
            let viewing = purpose == crate::core::sites::view::PURPOSE;
            if purpose != passkeys::REGISTER
                && purpose != passkeys::ADD
                && !purpose.starts_with("passkey:remove:")
                && !purpose.starts_with("grant:")
                && !purpose.starts_with("device:")
                && !viewing
            {
                return refused(400, "no such purpose");
            }
            let mut asked = json!({ "purpose": purpose });
            let shown = body["page"]
                .as_str()
                .filter(|_| purpose != passkeys::REGISTER);
            if let Some(shown) = shown
                && let Some((fixed, bound)) = bound_for(env, s, shown).await?
            {
                if fixed.is_some_and(|f| f != purpose) {
                    return refused(400, "this page isn't that grant's");
                }
                asked["bound"] = json!(bound);
            }
            let mut v = to_instance(env, s, "/passkeys/challenge", &asked).await?;
            v["rp"] = json!(rp_for(env, &host));
            Response::from_json(&v)
        }
        (Method::Get, "/passkeys") => list(env, s, None).await,
        (Method::Post, "/passkeys") => {
            let form = req.text().await?;
            let f = |k: &str| page::field(&form, k).unwrap_or_default();
            let said = if let Some(id) = page::field(&form, "remove") {
                let purpose = passkeys::remove_purpose(&id);
                if let Err(why) = on_page(env, s, &form, &purpose, &host, "/passkeys").await? {
                    return refused(403, &why);
                }
                let asked = json!({ "id": id, "min": MIN_KEYS });
                let v = to_instance(env, s, "/passkeys/remove", &asked).await?;
                said_of(&v, "Removed.")
            } else if page::field(&form, "add").is_some() {
                if !answered(&form) {
                    return refused(403, "adding a passkey this way takes an existing passkey");
                }
                let proved = prove_with(env, s, &form, passkeys::ADD, &host, "/passkeys").await?;
                if let Err(why) = proved {
                    return refused(403, &why);
                }
                to_instance(env, s, "/passkeys/allow", &json!({})).await?;
                "Proven. Create the new passkey within five minutes.".to_string()
            } else {
                let code = f("code");
                let coded = !code.trim().is_empty();
                if coded && let Err(why) = elicit::check_code(env, s, code.trim()).await? {
                    return refused(403, &why);
                }
                let v = to_instance(
                    env,
                    s,
                    "/passkeys/register",
                    &json!({
                        "id": f("pk_id"), "name": f("name"), "client": f("pk_client"),
                        "authenticator": f("pk_authenticator"), "spki": f("pk_spki"), "host": host,
                        "rp": rp_for(env, &host), "coded": coded,
                    }),
                )
                .await?;
                said_of(&v, "Registered. It now approves on these pages.")
            };
            list(env, s, Some(&said)).await
        }
        _ => refused(404, "not found"),
    }
}

/// What the page says of the `Instance`'s answer `v`: `done`, or why it was refused.
fn said_of(v: &Value, done: &str) -> String {
    if v["status"] == "ok" {
        done.to_string()
    } else {
        format!("Refused: {}", v["why"].as_str().unwrap_or("unknown"))
    }
}

async fn list(env: &Env, s: &Settings, said: Option<&str>) -> Result<Response> {
    let v = to_instance(env, s, "/passkeys/list", &json!({})).await?;
    let keys: Vec<page::Listed> = v["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| page::Listed {
            id: k["id"].as_str().unwrap_or_default().to_string(),
            name: k["name"].as_str().unwrap_or_default().to_string(),
            created: k["created"].as_u64().unwrap_or_default(),
            used: k["used"].as_u64(),
            synced: k["synced"].as_bool(),
        })
        .collect();
    let adding = v["adding"].as_bool().unwrap_or(false);
    html_with(
        200,
        &page::passkeys_page(&keys, said, adding),
        page::PROOF_CSP,
    )
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[
    super::Route::any(crate::routes::Path::Exact("/passkeys"), serve_route),
    super::Route::any(
        crate::routes::Path::Exact("/passkeys/challenge"),
        serve_route,
    ),
    super::Route::any(crate::routes::Path::Exact("/passkey.js"), serve_route),
];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
}
