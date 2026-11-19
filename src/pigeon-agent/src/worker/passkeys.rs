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
use crate::core::approval::enroll::Enrolments;
use crate::core::approval::passkeys::{self, Bound, Keyring, Proofs};
use crate::core::approval::totp::Guard;
use crate::core::platform::platforms::{Run, Stage};
use crate::core::pool::resume;
use crate::push::{b64url, b64url_decode};

/// Where the `Instance` keeps the keyring.
pub(super) const KEYRING: &str = "passkeys";

/// Where the `Instance` keeps until when a proven owner may add a passkey.
const ADDING: &str = "passkey-adding";

/// The grant pages whose grant the `Instance` keeps: each path prefix, its peek, and the
/// operation its proof is bound to.
const INSTANCE_PAGES: [(&str, &str, &str); 6] = [
    ("/mandate/", "/mandates/peek", "mandate.grant"),
    ("/lane/", "/lanes/peek", "lane.grant"),
    ("/grants/", "/grant-requests/peek", "grant.request"),
    ("/ci-window/", "/ci-window/peek", "ci.window"),
    ("/service-bind/", "/members/bind/peek", "service.bind"),
    ("/member-grant/", "/members/grant/peek", "member.grant"),
];

/// Where the `Instance` keeps what it has seen proven with a passkey.
pub(super) const PROOFS: &str = "passkey-proofs";

/// The Worker var that asks for the cutover, and the value that does.
pub(super) const PROOFS_VAR: &str = "PROOFS";
/// What [`PROOFS_VAR`] holds once codes are to be refused.
pub(super) const PROOFS_PASSKEY: &str = "passkey";

/// Where the `Instance` keeps the enrolments used and the wrong tokens tried.
const ENROLMENTS: &str = "passkey-enrolments";

/// The Worker secret an enrolment's token is checked against.
const ENROLL_SECRET: &str = "PASSKEY_ENROLL";

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
            let proofs = gate(state, &ring, &body).await?;
            let listed = json!({
            "keys": ring.keys.iter().map(|k| json!({
                "id": k.id, "name": k.name, "created": k.created, "used": k.used,
                "synced": k.synced,
            })).collect::<Vec<_>>(),
            "adding": adding > now(),
            "proofs": {
                "missing": proofs.missing(&ring, now()), "cutover": proofs.cutover,
            },
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
            let coded = body["coded"].as_bool() == Some(true);
            if !coded {
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
            if registered.is_ok() && !coded {
                seen(state, "added").await?;
            }
            (ring, outcome(registered))
        }
        "/passkeys/cutover" => {
            let proofs = gate(state, &ring, &body).await?;
            return Response::from_json(&json!({ "made": proofs.cutover.is_some() }));
        }
        "/passkeys/prove" => proving(state, ring, &body, &rp).await?,
        "/passkeys/enroll" => return enrolling(state, ring, &body, &rp).await,
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
            Ok(evidence) => {
                seen(state, &s("surface")).await?;
                json!({ "status": "ok", "evidence": evidence })
            }
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
    if proved.is_ok() {
        seen(state, &s("surface")).await?;
    }
    Ok((ring, outcome(proved)))
}

/// `/passkeys/enroll`: redeems `body.token` against `body.secret`, then registers the passkey,
/// after dropping the others when `body.replace`; nothing is kept unless both pass.
async fn enrolling(state: &State, ring: Keyring, body: &Value, rp: &str) -> Result<Response> {
    let storage = state.storage();
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();
    let bytes = |k: &str| b64url_decode(body[k].as_str().unwrap_or_default()).unwrap_or_default();
    let all: Enrolments = storage.get(ENROLMENTS).await?.unwrap_or_default();
    let (all, redeemed) = all.redeem(body["secret"].as_str(), &s("token"), now());
    if redeemed.is_err() {
        storage.put(ENROLMENTS, &all).await?;
        return Response::from_json(&outcome(redeemed));
    }
    let ring = if body["replace"].as_bool() == Some(true) {
        ring.cleared()
    } else {
        ring
    };
    let (ring, registered) = ring.register(
        &s("id"),
        &s("name"),
        &bytes("client"),
        &bytes("authenticator"),
        &bytes("spki"),
        &s("host"),
        rp,
        now(),
    );
    if registered.is_ok() {
        storage.put(ENROLMENTS, &all).await?;
        storage.put(KEYRING, &ring).await?;
        seen(state, "enrolled").await?;
    }
    Response::from_json(&outcome(registered))
}

/// The proofs after the gate: the cutover made and recorded when `body.asked` and nothing is
/// missing; see `.claude/decisions/2026-11-14-security-passkey-only-owner-proofs.md` §4.
async fn gate(state: &State, ring: &Keyring, body: &Value) -> Result<Proofs> {
    let storage = state.storage();
    let proofs: Proofs = storage.get(PROOFS).await?.unwrap_or_default();
    let asked = body["asked"].as_bool() == Some(true);
    let (next, _) = proofs.cutover(asked, ring, now());
    if next != proofs {
        storage.put(PROOFS, &next).await?;
    }
    Ok(next)
}

/// Whether [`PROOFS_VAR`] asks for the cutover.
fn asked(env: &Env) -> bool {
    env.var(PROOFS_VAR)
        .is_ok_and(|v| v.to_string() == PROOFS_PASSKEY)
}

/// Records `surface` as proven with a passkey now; an empty or unknown one records nothing.
async fn seen(state: &State, surface: &str) -> Result<()> {
    if surface.is_empty() {
        return Ok(());
    }
    let storage = state.storage();
    let proofs: Proofs = storage.get(PROOFS).await?.unwrap_or_default();
    storage.put(PROOFS, &proofs.seen(surface, now())).await
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
            "surface": if purpose.starts_with("grant:") { "app" } else { "" },
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
    if ["/passkeys", "/devices", "/app/link"].contains(&path) {
        return Ok(Some((None, Bound::default())));
    }
    if let Some(n) = path
        .strip_prefix("/approve/")
        .and_then(|n| n.parse::<u32>().ok())
    {
        let body = json!({ "token": token });
        let (status, v) =
            super::to_change(env, &s.repo, n, Method::Post, "/peek", Some(&body)).await?;
        let Ok(grant) =
            serde_json::from_value::<crate::core::approval::grants::Grant>(v["grant"].clone())
        else {
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
    let shown = |operation: &str, id: &str, v: &Value| {
        let named = v["id"].as_u64().map(|n| n.to_string());
        (v.is_object() && named.as_deref() == Some(id)).then(|| {
            let bound = Bound {
                operation: operation.to_string(),
                target: id.to_string(),
                head: crate::core::approval::grants::hex(&sha2::Sha256::digest(
                    v.to_string().as_bytes(),
                )),
            };
            (purpose.clone(), bound)
        })
    };
    for (prefix, peek, operation) in INSTANCE_PAGES {
        if let Some(id) = path.strip_prefix(prefix) {
            let v = to_instance(env, s, peek, &json!({ "token": token })).await?;
            return Ok(shown(operation, id, &v));
        }
    }
    if let Some(id) = path.strip_prefix("/platform/") {
        let Ok(n) = id.parse::<u32>() else {
            return Ok(None);
        };
        let asked = json!({ "id": n, "token": token });
        let (status, v) = super::platforms::to_main(env, s, "peek", &asked).await?;
        let mut found = shown("platform.run", id, &v).filter(|_| status == 200);
        // A plan that mints its token is proven for the mint's purpose, as its page asks.
        if let (Some((purpose, _)), Ok(run)) = (found.as_mut(), serde_json::from_value::<Run>(v))
            && run.stage == Stage::Offered
            && let (Ok(wanted), true) = super::platforms::terms(env, s, &run).await
        {
            let names: Vec<String> = wanted.iter().map(|w| w.name.clone()).collect();
            *purpose = Some(passkeys::mint_purpose(&token, &names));
        }
        return Ok(found);
    }
    if let Some((name, id)) = resume::parse(path) {
        let peek = json!({ "token": token });
        let (status, v) = super::pool::to_pool(env, name, "/resume/peek", &peek).await?;
        return Ok(shown(resume::OPERATION, &id.to_string(), &v).filter(|_| status == 200));
    }
    Ok(None)
}

/// What the page at `page` shows, as `present::asked` builds it from the page's own peek; `None`
/// for a page it doesn't know or a grant no longer open; see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its 2026-11-18 amendment §2.
async fn asked_for(env: &Env, s: &Settings, page: &str) -> Result<Option<Value>> {
    use crate::core::present::asked;
    let (path, query) = page.split_once('?').unwrap_or((page, ""));
    let token = page::field(query, "grant").unwrap_or_default();
    let peek = json!({ "token": token });
    if let Some(n) = path
        .strip_prefix("/approve/")
        .and_then(|n| n.parse::<u32>().ok())
    {
        let (_, v) = super::to_change(env, &s.repo, n, Method::Post, "/peek", Some(&peek)).await?;
        let change = serde_json::from_value::<crate::core::change::Change>(v["change"].clone());
        let grant =
            serde_json::from_value::<crate::core::approval::grants::Grant>(v["grant"].clone());
        return Ok(match (change, grant) {
            (Ok(c), Ok(g)) => Some(asked::change(&c, &g)),
            _ => None,
        });
    }
    for (prefix, route, _) in INSTANCE_PAGES {
        if !path.starts_with(prefix) {
            continue;
        }
        let v = to_instance(env, s, route, &peek).await?;
        return Ok(match prefix {
            "/mandate/" => serde_json::from_value(v).ok().map(|m| asked::mandate(&m)),
            "/lane/" => serde_json::from_value(v).ok().map(|l| asked::lane(&l)),
            "/ci-window/" => serde_json::from_value(v).ok().map(|o| asked::ci_window(&o)),
            "/service-bind/" => serde_json::from_value(v)
                .ok()
                .map(|o| asked::service_bind(&o)),
            "/member-grant/" => serde_json::from_value(v)
                .ok()
                .map(|o| asked::member_grant(&o)),
            "/grants/" => {
                let Ok(o) = serde_json::from_value::<crate::core::minting::Offer>(v) else {
                    return Ok(None);
                };
                let entry = super::minting::deployed(env, s)
                    .await?
                    .and_then(|r| r.ledger.grants.get(&o.name).cloned())
                    .or_else(|| o.spec.clone());
                Some(asked::grant(&o, entry.as_ref()))
            }
            _ => None,
        });
    }
    if let Some(id) = path
        .strip_prefix("/platform/")
        .and_then(|n| n.parse::<u32>().ok())
    {
        let (status, v) =
            super::platforms::to_main(env, s, "peek", &json!({ "id": id, "token": token })).await?;
        let Ok(run) = serde_json::from_value::<Run>(v) else {
            return Ok(None);
        };
        if status != 200 {
            return Ok(None);
        }
        let (needs, mint) = if run.stage == Stage::Offered {
            super::platforms::terms(env, s, &run).await
        } else {
            (Err(String::new()), false)
        };
        return Ok(Some(asked::platform(&run, &needs, mint)));
    }
    if let Some((name, _)) = resume::parse(path) {
        let (_, v) = super::pool::to_pool(env, name, "/resume/peek", &peek).await?;
        return Ok(serde_json::from_value(v)
            .ok()
            .map(|o| asked::pool_resume(&o)));
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
    if path.starts_with("/approve/") {
        asked["surface"] = json!("page");
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

// The proof tools; see `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md` §4.

/// `proof.challenge {page}`: what the grant at `page` approves, from the grant, and the `WebAuthn`
/// request options for its bound challenge; a refusal when `page` holds no open grant.
///
/// # Errors
///
/// When a binding fails.
pub async fn proof_challenge(
    env: &Env,
    s: &Settings,
    args: &Value,
    host: &str,
) -> Result<std::result::Result<Value, String>> {
    if let Some((purpose, asked)) = account_purpose(args) {
        let v = to_instance(
            env,
            s,
            "/passkeys/challenge",
            &json!({ "purpose": purpose }),
        )
        .await?;
        return Ok(Ok(json!({
            "purpose": purpose,
            "operation": asked["submit"]["tool"],
            "target": args["target"],
            "challenge": v["challenge"],
            "ids": v["ids"],
            "user": v["user"],
            "rp": super::app::relying_party(env),
            "asked": asked,
        })));
    }
    if let Some(p) = args["purpose"].as_str() {
        let Some((purpose, operation)) = auto_purpose(p) else {
            return Ok(Err(
                "`purpose` is auto, auto-end, unlink, passkey-remove or instance-standup".into(),
            ));
        };
        let v = to_instance(
            env,
            s,
            "/passkeys/challenge",
            &json!({ "purpose": purpose }),
        )
        .await?;
        return Ok(Ok(json!({
            "purpose": purpose,
            "operation": operation,
            "target": "auto",
            "challenge": v["challenge"],
            "ids": v["ids"],
            "user": v["user"],
            "rp": super::app::relying_party(env),
        })));
    }
    let page = args["page"].as_str().unwrap_or_default();
    let Some((Some(purpose), bound)) = bound_for(env, s, page).await? else {
        return Ok(Err("that page holds no open grant".into()));
    };
    let asked = json!({ "purpose": purpose, "bound": &bound });
    let v = to_instance(env, s, "/passkeys/challenge", &asked).await?;
    let mut answer = json!({
        "page": page,
        "operation": bound.operation,
        "target": bound.target,
        "head": bound.head,
        "purpose": purpose,
        "challenge": v["challenge"],
        "ids": v["ids"],
        "user": v["user"],
        "rp": rp_for(env, host),
    });
    if let Some(shown) = asked_for(env, s, page).await? {
        answer["asked"] = shown;
    }
    Ok(Ok(answer))
}

/// `proof.submit`: the passkey assertion in `args`, with the page's other `fields`, posted to the
/// page at `args.page` as its own form, so [`on_page`] checks it and the page redeems its grant
/// and runs its operation. The assertion goes nowhere else.
///
/// # Errors
///
/// When a binding fails.
pub async fn proof_submit(
    env: &Env,
    s: &Settings,
    who: &Identity,
    args: &Value,
    host: &str,
) -> Result<std::result::Result<Value, String>> {
    let text = |k: &str| args[k].as_str().unwrap_or_default();
    if let Some(p) = args["purpose"].as_str() {
        if !matches!(p, "auto" | "auto-end") {
            return Ok(Err("`purpose` is auto or auto-end".into()));
        }
        let body = json!({ "passkey": {
            "id": text("credential"), "client": text("client_data"),
            "authenticator": text("authenticator_data"), "signature": text("signature"),
        } });
        let mut r = if p == "auto" {
            super::app::auto_start(env, s, &body).await?
        } else {
            super::app::auto_end(env, s, &body).await?
        };
        let status = r.status_code();
        return Ok(if status == 200 {
            let v: Value = r.json().await?;
            Ok(json!({ "purpose": p, "status": status, "window": v["window"]["id"] }))
        } else {
            Err(format!(
                "refused ({status}): {}",
                r.text().await.unwrap_or_default()
            ))
        });
    }
    let page = text("page");
    let (path, query) = page.split_once('?').unwrap_or((page, ""));
    let Some(token) = page::field(query, "grant").filter(|t| !t.is_empty()) else {
        return Ok(Err("`page` names no grant".into()));
    };
    if !matches!(bound_for(env, s, page).await?, Some((Some(_), _))) {
        return Ok(Err("that page holds no open grant".into()));
    }
    let mut pairs = vec![("grant".to_string(), token)];
    // The page's fields: `fields`, and any other top-level argument (a view's action or form);
    // a true toggle posts "1", a false one nothing.
    let top = args
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !TOOL_ARGS.contains(&k.as_str()));
    let nested = args["fields"].as_object().into_iter().flatten();
    for (k, v) in nested.chain(top) {
        let ours = k == "grant" || k == "code" || k.starts_with("pk_");
        let value = match v {
            Value::String(t) => Some(t.clone()),
            Value::Bool(true) => Some("1".to_string()),
            _ => None,
        };
        if let (false, Some(value)) = (ours, value) {
            pairs.push((k.clone(), value));
        }
    }
    for (k, from) in [
        ("pk_id", "credential"),
        ("pk_client", "client_data"),
        ("pk_authenticator", "authenticator_data"),
        ("pk_signature", "signature"),
    ] {
        pairs.push((k.to_string(), text(from).to_string()));
    }
    let form = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let mut r = super::page_post(env, s, who, page, &form, host).await?;
    let status = r.status_code();
    let said = plain(&r.text().await.unwrap_or_default());
    Ok(if status == 200 {
        Ok(json!({ "page": path, "status": status, "said": said }))
    } else {
        Err(format!("refused ({status}): {said}"))
    })
}

/// `proof.submit`'s own arguments; every other top-level argument is one of the page's fields.
const TOOL_ARGS: [&str; 7] = [
    "page",
    "purpose",
    "fields",
    "credential",
    "client_data",
    "authenticator_data",
    "signature",
];

/// `proof.challenge`'s account purposes: `unlink` and `passkey-remove` with a `target` id, as
/// the passkey purpose `/v0/app/devices/<id>` and `/v0/app/passkeys/<id>` prove, and the card
/// whose `submit` redeems it as the device operation; `instance-standup` with the token's
/// SHA-256 as `target`, whose card has no submit (the app signs it); see
/// `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its R8 and R8c amendments.
fn account_purpose(args: &Value) -> Option<(String, Value)> {
    use crate::core::account;
    let target = args["target"].as_str().filter(|t| !t.is_empty())?;
    if args["purpose"].as_str()? == "instance-standup" {
        let asked = json!({
            "title": "Stand up this instance",
            "body": "The app's stand-up sheet signs this, then sends the account token once.",
            "verb": "Stand up", "fields": [], "inputs": [], "refuse": false, "change": null,
        });
        let sha = target.to_ascii_lowercase();
        return Some((crate::core::member::listing::standup_purpose(&sha), asked));
    }
    let (purpose, title, verb, tool) = match args["purpose"].as_str()? {
        "unlink" => (
            account::unlink_purpose(target),
            "Unlink this device",
            "Unlink",
            "device.unlink",
        ),
        "passkey-remove" => (
            account::passkey_remove_purpose(target),
            "Remove this passkey",
            "Remove",
            "passkey.remove",
        ),
        _ => return None,
    };
    let asked = json!({
        "title": title,
        "body": format!("{target} stops proving the owner's account."),
        "verb": verb,
        "fields": [],
        "inputs": [],
        "refuse": false,
        "change": null,
        "submit": {
            "label": verb, "tool": tool, "arguments": { "id": target },
            "style": "destructive", "proof": true,
        },
    });
    Some((purpose, asked))
}

/// A proof tool's `purpose`: the Auto window's proof purpose and the operation it stands for.
fn auto_purpose(p: &str) -> Option<(&'static str, &'static str)> {
    use crate::core::approval::windows;
    match p {
        "auto" => Some((windows::START, "auto.start")),
        "auto-end" => Some((windows::END, "auto.end")),
        _ => None,
    }
}

/// `s` encoded as a form value: unreserved bytes as they are, every other byte as `%XX`.
fn form_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A page's text without its markup, its whitespace collapsed, at most 400 characters.
fn plain(html: &str) -> String {
    let body = html.split_once("<body").map_or(html, |(_, b)| b);
    let mut out = String::new();
    let mut tag = false;
    for c in body.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ');
            }
            c if !tag => out.push(c),
            _ => {}
        }
    }
    let words = out.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(400).collect()
}

/// Whether the cutover is made: codes are refused, and a passkey is the only proof.
///
/// # Errors
///
/// When the binding fails.
pub(super) async fn cutover_made(env: &Env, s: &Settings) -> Result<bool> {
    let v = to_instance(env, s, "/passkeys/cutover", &json!({ "asked": asked(env) })).await?;
    Ok(v["made"].as_bool() == Some(true))
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
                // A platform plan's proof may also bind the token it mints; `decide` checks
                // that digest against the set it mints, per the minted-token ADR §4.
                if fixed
                    .is_some_and(|f| f != purpose && !purpose.starts_with(&format!("{f}:mint:")))
                {
                    return refused(400, "this page isn't that grant's");
                }
                asked["bound"] = json!(bound);
            }
            let mut v = to_instance(env, s, "/passkeys/challenge", &asked).await?;
            v["rp"] = json!(rp_for(env, &host));
            Response::from_json(&v)
        }
        (Method::Get, "/passkeys") => list(env, s, None).await,
        (Method::Get, "/passkeys/enroll") => {
            html_with(200, &page::enroll_page(None), page::PROOF_CSP)
        }
        (Method::Post, "/passkeys/enroll") => {
            let form = req.text().await?;
            let said = enrol(env, s, who, &form, &host).await?;
            html_with(200, &page::enroll_page(Some(&said)), page::PROOF_CSP)
        }
        (Method::Post, "/passkeys") => {
            let form = req.text().await?;
            managing(env, s, &form, &host).await
        }
        _ => refused(404, "not found"),
    }
}

/// `POST /passkeys`: removes a passkey, proves the owner so one may be added, or registers one.
async fn managing(env: &Env, s: &Settings, form: &str, host: &str) -> Result<Response> {
    let f = |k: &str| page::field(form, k).unwrap_or_default();
    let said = if let Some(id) = page::field(form, "remove") {
        let purpose = passkeys::remove_purpose(&id);
        if let Err(why) = on_page(env, s, form, &purpose, host, "/passkeys").await? {
            return refused(403, &why);
        }
        let asked = json!({ "id": id, "min": MIN_KEYS });
        let v = to_instance(env, s, "/passkeys/remove", &asked).await?;
        said_of(&v, "Removed.")
    } else if page::field(form, "add").is_some() {
        if !answered(form) {
            return refused(403, "adding a passkey this way takes an existing passkey");
        }
        let proved = prove_with(env, s, form, passkeys::ADD, host, "/passkeys").await?;
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
                "rp": rp_for(env, host), "coded": coded,
            }),
        )
        .await?;
        said_of(&v, "Registered. It now approves on these pages.")
    };
    list(env, s, Some(&said)).await
}

/// Enrols the passkey posted as `form` with its token: what the page says. An enrolment is
/// logged and pushed.
async fn enrol(env: &Env, s: &Settings, who: &Identity, form: &str, host: &str) -> Result<String> {
    let f = |k: &str| page::field(form, k).unwrap_or_default();
    let replace = f("replace") == "1";
    let asked = json!({
        "secret": platform::secret(env, ENROLL_SECRET).ok(), "token": f("token"),
        "replace": replace, "id": f("pk_id"), "name": f("name"), "client": f("pk_client"),
        "authenticator": f("pk_authenticator"), "spki": f("pk_spki"), "host": host,
        "rp": rp_for(env, host),
    });
    let v = to_instance(env, s, "/passkeys/enroll", &asked).await?;
    if v["status"] == "ok" {
        let said = format!(
            "a passkey was enrolled by break-glass by {}{}",
            who.subject,
            if replace {
                ", replacing every other"
            } else {
                ""
            }
        );
        let _ = super::events::append(env, s, "passkey.enrolled", None, None, &said, true).await;
        super::push::notify(env, s, "Passkey enrolled", &said, "/passkeys").await;
    }
    Ok(said_of(
        &v,
        "Enrolled. This passkey now approves on these pages.",
    ))
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
    let v = to_instance(env, s, "/passkeys/list", &json!({ "asked": asked(env) })).await?;
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
    let missing: Vec<String> =
        serde_json::from_value(v["proofs"]["missing"].clone()).unwrap_or_default();
    let cutover = page::Cutover {
        missing: &missing,
        since: v["proofs"]["cutover"].as_u64(),
    };
    html_with(
        200,
        &page::passkeys_page(&keys, said, adding, &cutover),
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
    super::Route::any(crate::routes::Path::Exact("/passkeys/enroll"), serve_route),
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
