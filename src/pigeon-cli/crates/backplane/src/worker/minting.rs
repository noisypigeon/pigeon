//! Grants in the control plane: `grant.*`, the `Instance` object's requests and registry, and
//! `/grants/<id>`, the page that runs a grant on the owner's code; values never leave the
//! Worker; see `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use worker::{Env, Method, Request, Response, Result, State};

use super::mandates::to_instance;
use super::platform::{self, Settings, json_request, now};
use super::{approve as page, elicit};
use crate::access::Identity;
use crate::approve::{self, Waiting};
use crate::core::ledger::{self, Grant, KeyKind};
use crate::core::minting::{
    self, Group, Kind, Offer, Record, Registry, Requests, Resolved, Settled,
};

/// The storage key of the requests waiting for the owner.
const REQUESTS: &str = "grant-requests";

/// The storage key of each grant's record.
const REGISTRY: &str = "grant-registry";

/// The Secrets Store binding holding `granter`.
const GRANTER: &str = "GRANTER";

/// How long a helper token lives, in seconds.
const HELPER_TTL: u64 = 300;

/// The permission a helper token puts Worker secrets with.
const SCRIPTS_WRITE: &str = "Workers Scripts Write";

/// `operation` on grant `name`: the list, or a request waiting for the owner, with its link.
///
/// # Errors
///
/// When a binding fails.
pub async fn run(env: &Env, s: &Settings, operation: &str, name: &str) -> Result<(u16, Value)> {
    let resolved = deployed(env, s).await?;
    let registry = registry(env, s).await?;
    if operation == "grant.list" {
        return Ok((
            200,
            json!({ "ledger": resolved.as_ref().map(|r| &r.ledger), "sha": resolved.as_ref().map(|r| &r.sha), "grants": registry.grants }),
        ));
    }
    let kind = match operation {
        "grant.request" => Kind::Request,
        "grant.rotate" => Kind::Rotate,
        "grant.revoke" => Kind::Revoke,
        other => return Ok((404, json!(format!("`{other}` is not a grant operation")))),
    };
    let declared = resolved
        .as_ref()
        .is_some_and(|r| r.ledger.grants.contains_key(name));
    let made = registry.grants.contains_key(name);
    let refused = match kind {
        Kind::Request | Kind::Rotate if !declared => Some(format!(
            "`{name}` is not in the deployed ledger; propose it in .config/grants.yaml first"
        )),
        Kind::Rotate | Kind::Revoke if !made => Some(format!("`{name}` was never granted")),
        _ => None,
    };
    if let Some(why) = refused {
        return Ok((409, json!(why)));
    }
    let token = platform::random_hex(32)?;
    let expires = now() + crate::grants::PAGE_TTL;
    let body = json!({ "name": name, "kind": kind, "token": token, "expires": expires });
    let (status, v) = to_instance(env, s, "/grant-requests/offer", &body).await?;
    if status != 200 {
        return Ok((status, v));
    }
    let offer: Offer = serde_json::from_value(v)?;
    let path = format!("/grants/{}?grant={token}", offer.id);
    let title = format!("{} grant `{name}`", verb(kind));
    let waiting = Waiting {
        number: offer.id,
        operation: operation.into(),
        title: title.clone(),
        path: path.clone(),
        expires,
    };
    elicit::inbox(env, s, "/approvals/add", &json!(waiting)).await?;
    let said = format!("`{operation}` ({title}) waits for the owner");
    let _ = super::events::append(env, s, "grant.requested", None, None, &said, false).await;
    super::push::notify(env, s, "Approval waiting", &said, &path).await;
    let host = env
        .var(crate::cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((
        202,
        json!({ "pending": { "operation": operation, "link": format!("https://{host}{path}") } }),
    ))
}

fn verb(kind: Kind) -> &'static str {
    match kind {
        Kind::Request => "Make",
        Kind::Rotate => "Rotate",
        Kind::Revoke => "Revoke",
    }
}

/// The ledger the last passed deploy reported, if any.
async fn deployed(env: &Env, s: &Settings) -> Result<Option<Resolved>> {
    let req = json_request("https://main/ledger", Method::Get, None, None)?;
    let mut r = platform::call(env, &super::main_stub(env, &s.repo)?, req).await?;
    if r.status_code() != 200 {
        return Ok(None);
    }
    Ok(r.json().await.ok())
}

async fn registry(env: &Env, s: &Settings) -> Result<Registry> {
    let (_, v) = to_instance(env, s, "/grant-registry/get", &json!({})).await?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The `Instance` Durable Object's side: `/grant-requests/offer|peek|redeem` and
/// `/grant-registry/get|put|remove`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let token = body["token"].as_str().unwrap_or_default();
    if let Some(op) = path.strip_prefix("/grant-registry/") {
        let mut r: Registry = storage.get(REGISTRY).await?.unwrap_or_default();
        let name = body["name"].as_str().unwrap_or_default().to_string();
        match op {
            "get" => return Response::from_json(&r),
            "put" => {
                let record: Record = serde_json::from_value(body["record"].clone())?;
                r.grants.insert(name, record);
            }
            "remove" => {
                r.grants.remove(&name);
            }
            _ => return Response::error("not found", 404),
        }
        storage.put(REGISTRY, &r).await?;
        return Response::from_json(&r);
    }
    let mut all: Requests = storage.get(REQUESTS).await?.unwrap_or_default();
    let answer = match path {
        "/grant-requests/peek" => {
            return match all.peek(token, now()) {
                Ok(o) => Response::from_json(o),
                Err(why) => Response::error(why, 403),
            };
        }
        "/grant-requests/offer" => {
            let kind: Kind = serde_json::from_value(body["kind"].clone())?;
            let name = body["name"].as_str().unwrap_or_default();
            let expires = body["expires"].as_u64().unwrap_or_default();
            Ok(json!(all.offer(name, kind, None, token, now(), expires)))
        }
        "/grant-requests/redeem" => all.redeem(token, now()).map(|o| json!(o)),
        _ => return Response::error("not found", 404),
    };
    storage.put(REQUESTS, &all).await?;
    match answer {
        Ok(v) => Response::from_json(&v),
        Err(why) => Response::error(why, 409),
    }
}

/// The id in `/grants/<id>`.
fn number(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/grants/")?;
    (!n.starts_with('0')).then(|| n.parse().ok()).flatten()
}

/// Serves `/grants/<id>` for `who`: the page, and the POST that runs it with the code.
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
    if !who.human {
        return page::refused(403, "only a person signed in to Access grants a secret");
    }
    let Some(id) = number(&req.path()) else {
        return page::refused(404, "not found");
    };
    let url = req.url()?;
    let host = url.host_str().unwrap_or_default().to_string();
    match req.method() {
        Method::Get => {
            let Some(token) = url.query().and_then(|q| approve::field(q, "grant")) else {
                return page::refused(400, "this link has no grant");
            };
            let (status, v) =
                to_instance(env, s, "/grant-requests/peek", &json!({ "token": token })).await?;
            let Ok(o) = serde_json::from_value::<Offer>(v.clone()) else {
                return page::refused(403, v.as_str().unwrap_or("this link is not valid"));
            };
            if status != 200 || o.id != id {
                return page::refused(403, "this link is not valid");
            }
            let entry = deployed(env, s)
                .await?
                .and_then(|r| r.ledger.grants.get(&o.name).cloned());
            page::html(200, &approve::grant_page(&o, entry.as_ref(), &token))
        }
        Method::Post => {
            let origin = req.headers().get("origin")?;
            let site = req.headers().get("sec-fetch-site")?;
            if !approve::same_origin(origin.as_deref(), site.as_deref(), &host) {
                return page::refused(403, "this form is posted only from its own page");
            }
            let form = req.text().await?;
            let Some(token) = approve::field(&form, "grant") else {
                return page::refused(400, "the form is missing its grant");
            };
            let code = approve::field(&form, "code").unwrap_or_default();
            if let Err(why) = elicit::check_code(env, s, code.trim()).await? {
                return page::refused(403, &why);
            }
            let (status, v) =
                to_instance(env, s, "/grant-requests/redeem", &json!({ "token": token })).await?;
            let _ = elicit::inbox(
                env,
                s,
                "/approvals/remove",
                &json!({ "path": format!("/grants/{id}?grant={token}") }),
            )
            .await;
            let Ok(o) = serde_json::from_value::<Offer>(v.clone()) else {
                return page::refused(409, v.as_str().unwrap_or("this link is not valid"));
            };
            if status != 200 {
                return page::refused(409, "this link is not valid");
            }
            match execute(env, s, &o, &who.subject).await? {
                Ok(said) => page::html(200, &approve::outcome("Done", &said)),
                Err(why) => {
                    let said = format!("{} grant `{}` failed: {why}", verb(o.kind), o.name);
                    let _ = super::events::append(env, s, "grant.failed", None, None, &said, false)
                        .await;
                    let _ = super::tasks::add(env, s, "grant-failed", 0, None, None, &said).await;
                    page::refused(409, &said)
                }
            }
        }
        _ => page::refused(405, "method not allowed"),
    }
}

/// Runs request `o` as `by`: what was done, or why nothing was left done.
async fn execute(
    env: &Env,
    s: &Settings,
    o: &Offer,
    by: &str,
) -> Result<std::result::Result<String, String>> {
    let Some(resolved) = deployed(env, s).await? else {
        return Ok(Err("no deploy has reported the grants ledger yet".into()));
    };
    let registry = registry(env, s).await?;
    let granter = match granter(env).await {
        Ok(g) => g,
        Err(why) => return Ok(Err(why)),
    };
    let account = match platform::account(&granter).await {
        Ok(a) => a,
        Err(e) => return Ok(Err(format!("`granter` sees no single account: {e}"))),
    };
    let api = Api {
        granter: &granter,
        account: &account,
    };
    if o.kind == Kind::Revoke {
        return revoke(env, s, &api, &registry, &o.name, by).await;
    }
    let Some(grant) = resolved.ledger.grants.get(&o.name) else {
        return Ok(Err(format!("`{}` is not in the deployed ledger", o.name)));
    };
    let old = registry
        .grants
        .get(&o.name)
        .and_then(|r| r.token_id.clone());
    let (minted, value, capabilities, expires) = match make(&api, &resolved, &o.name, grant).await?
    {
        Ok(m) => m,
        Err(why) => return Ok(Err(why)),
    };
    // An error past the mint is the put's, so `settle` still revokes what was minted.
    let put = put(&api, grant, value.as_deref().unwrap_or_default())
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    let verified = match (&minted, &value) {
        (Some(_), Some(v)) => verify(&account, v).await,
        _ => Ok(()),
    };
    let settled = minting::settle(Ok(minted.clone()), put, verified, old.as_deref());
    match settled {
        Settled::Recorded { revoke } => {
            let record = Record {
                token_id: minted,
                route: grant.route.clone(),
                capabilities,
                made: now(),
                expires,
                by: by.to_string(),
            };
            to_instance(
                env,
                s,
                "/grant-registry/put",
                &json!({ "name": o.name, "record": record }),
            )
            .await?;
            if let Some(id) = revoke.filter(|id| Some(id) != record.token_id.as_ref()) {
                api.revoke(&id).await.map_err(|e| e.to_string())?;
            }
            let (kind, done) = if o.kind == Kind::Rotate {
                ("grant.rotated", "rotated")
            } else {
                ("grant.granted", "granted")
            };
            let said = format!(
                "`{}` {done} by {by}: {}{}",
                o.name,
                route_said(&grant.route),
                record
                    .token_id
                    .as_deref()
                    .map(|t| format!(", token {t}"))
                    .unwrap_or_default()
            );
            super::events::append(env, s, kind, None, None, &said, false).await?;
            Ok(Ok(said))
        }
        Settled::Undo { revoke, why } => {
            api.revoke(&revoke).await.map_err(|e| e.to_string())?;
            Ok(Err(format!("{why}; the new token was revoked")))
        }
        Settled::Failed { why } => Ok(Err(why)),
    }
}

/// Revokes grant `name`'s token and removes its value, as `by`.
async fn revoke(
    env: &Env,
    s: &Settings,
    api: &Api<'_>,
    registry: &Registry,
    name: &str,
    by: &str,
) -> Result<std::result::Result<String, String>> {
    let Some(record) = registry.grants.get(name) else {
        return Ok(Err(format!("`{name}` was never granted")));
    };
    if let Some(id) = &record.token_id {
        api.revoke(id).await.map_err(|e| e.to_string())?;
    }
    if let (Some(w), Some(k)) = (&record.route.worker, &record.route.key)
        && let Err(why) = api
            .helper_call(
                Method::Delete,
                &format!("/workers/scripts/{w}/secrets/{k}"),
                None,
            )
            .await?
    {
        return Ok(Err(format!(
            "the token is revoked, but {w}'s {k} stays: {why}"
        )));
    }
    to_instance(env, s, "/grant-registry/remove", &json!({ "name": name })).await?;
    let said = format!("`{name}` revoked by {by}");
    super::events::append(env, s, "grant.revoked", None, None, &said, false).await?;
    Ok(Ok(said))
}

fn route_said(r: &ledger::Route) -> String {
    match (&r.worker, &r.store, &r.job, &r.key) {
        (Some(w), _, _, Some(k)) => format!("put as {w}'s {k}"),
        (_, Some(st), _, Some(k)) => format!("put as {k} in store {st}"),
        (_, _, Some(j), _) => format!("for job {j}"),
        _ => "routed nowhere".into(),
    }
}

/// `granter`, from the Secrets Store binding.
async fn granter(env: &Env) -> std::result::Result<String, String> {
    let store = env
        .secret_store(GRANTER)
        .map_err(|_| format!("the Worker has no `{GRANTER}` binding yet"))?;
    store
        .get()
        .await
        .map_err(|e| format!("`granter` can't be read: {e}"))?
        .ok_or_else(|| "`granter` is not set in the Secrets Store".into())
}

/// The account API, as `granter`.
struct Api<'a> {
    granter: &'a str,
    account: &'a str,
}

impl Api<'_> {
    async fn groups(&self) -> Result<Vec<Group>> {
        let v = platform::cloudflare(
            self.granter,
            Method::Get,
            &format!("/accounts/{}/tokens/permission_groups", self.account),
            None,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Mints `body`: the token's ID and value.
    async fn mint(&self, body: &Value) -> Result<(String, String)> {
        let v = platform::cloudflare(
            self.granter,
            Method::Post,
            &format!("/accounts/{}/tokens", self.account),
            Some(body),
        )
        .await?;
        match (v["id"].as_str(), v["value"].as_str()) {
            (Some(id), Some(value)) => Ok((id.to_string(), value.to_string())),
            _ => Err("the minted token came back without its ID or value".into()),
        }
    }

    async fn revoke(&self, id: &str) -> Result<()> {
        platform::cloudflare(
            self.granter,
            Method::Delete,
            &format!("/accounts/{}/tokens/{id}", self.account),
            None,
        )
        .await
        .map(|_| ())
    }

    /// One account call through a [`HELPER_TTL`] token with [`SCRIPTS_WRITE`], revoked after.
    async fn helper_call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<std::result::Result<(), String>> {
        let groups = self.groups().await?;
        let wanted = [minting::Wanted {
            name: "helper".into(),
            permissions: vec![SCRIPTS_WRITE.into()],
            scope: "account".into(),
        }];
        let t = now();
        let spec = match minting::token_body(
            "pigeon grant helper",
            self.account,
            &wanted,
            &[],
            &BTreeMap::new(),
            &groups,
            &iso(t),
            &iso(t + HELPER_TTL),
        ) {
            Ok(b) => b,
            Err(p) => return Ok(Err(p.join("; "))),
        };
        let (id, value) = self.mint(&spec).await?;
        let done = platform::cloudflare(
            &value,
            method,
            &format!("/accounts/{}{path}", self.account),
            body,
        )
        .await;
        self.revoke(&id).await?;
        Ok(done.map(|_| ()).map_err(|e| e.to_string()))
    }
}

/// Makes grant `name`'s value: the token's ID (none for a generated key), the value, the
/// capabilities minted and the expiry; or why it can't be made yet.
type Made = (Option<String>, Option<String>, Vec<String>, Option<u64>);

async fn make(
    api: &Api<'_>,
    resolved: &Resolved,
    name: &str,
    grant: &Grant,
) -> Result<std::result::Result<Made, String>> {
    if let Some(m) = &grant.mint {
        let wanted = match resolved.wanted(name) {
            Ok(w) => w,
            Err(why) => return Ok(Err(why)),
        };
        let Some(ttl) = ledger::seconds(&m.ttl) else {
            return Ok(Err(format!("`{name}`'s ttl doesn't read")));
        };
        let groups = api.groups().await?;
        let t = now();
        let body = match minting::token_body(
            &format!("pigeon {name}"),
            api.account,
            &wanted,
            &m.zones,
            &BTreeMap::new(),
            &groups,
            &iso(t),
            &iso(t + ttl),
        ) {
            Ok(b) => b,
            Err(p) => return Ok(Err(p.join("; "))),
        };
        let (id, value) = api.mint(&body).await?;
        let caps = wanted.into_iter().map(|w| w.name).collect();
        return Ok(Ok((Some(id), Some(value), caps, Some(t + ttl))));
    }
    match grant.generate.as_ref().map(|g| g.kind) {
        Some(KeyKind::Random32 | KeyKind::Hmac) => Ok(Ok((
            None,
            Some(platform::random_hex(32)?),
            Vec::new(),
            None,
        ))),
        Some(k) => Ok(Err(format!(
            "`{name}` generates a {k:?} key, which a later change makes"
        ))),
        None => Ok(Err(format!(
            "`{name}` is derived, which a later change computes"
        ))),
    }
}

/// Puts `value` at grant's route.
async fn put(api: &Api<'_>, grant: &Grant, value: &str) -> Result<std::result::Result<(), String>> {
    match (&grant.route.worker, &grant.route.key) {
        (Some(w), Some(k)) => {
            let body = json!({ "name": k, "text": value, "type": "secret_text" });
            api.helper_call(
                Method::Put,
                &format!("/workers/scripts/{w}/secrets"),
                Some(&body),
            )
            .await
        }
        _ => Ok(Err(
            "store and job routes come with step 4's runs; only a Worker route is put yet".into(),
        )),
    }
}

/// Whether minted `value` is active on `account`.
async fn verify(account: &str, value: &str) -> std::result::Result<(), String> {
    let v = platform::cloudflare(
        value,
        Method::Get,
        &format!("/accounts/{account}/tokens/verify"),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    if v["status"].as_str() == Some("active") {
        Ok(())
    } else {
        Err("the new token isn't active".into())
    }
}

/// `secs` as RFC 3339, as the token API takes it.
fn iso(secs: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "seconds since 1970 fit an f64")]
    let ms = (secs * 1000) as f64;
    let iso: String = worker::js_sys::Date::new(&worker::wasm_bindgen::JsValue::from_f64(ms))
        .to_iso_string()
        .into();
    // The API takes whole seconds.
    iso.split_once('.')
        .map_or(iso.clone(), |(whole, _)| format!("{whole}Z"))
}

/// The routes this surface serves.
pub(super) const ROUTES: &[super::Route] = &[super::Route::any(
    crate::routes::Path::Prefix("/grants/"),
    serve_route,
)];

fn serve_route<'a>(
    req: Request,
    env: &'a Env,
    s: &'a Settings,
    who: &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>> {
    Box::pin(page_serve(req, env, s, who))
}
