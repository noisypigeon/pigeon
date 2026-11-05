//! What the Worker's handlers share: its settings, the Cloudflare API, Access's signature
//! check with `WebCrypto`, and D1.

use serde_json::Value;
use worker::js_sys::{self, Array, Function, Promise, Reflect, Uint8Array};
use worker::wasm_bindgen::{JsCast, JsValue};
use worker::wasm_bindgen_futures::JsFuture;
use worker::{Date, Env, Fetch, Headers, Method, Request, RequestInit, Result};

use crate::access::{self, Identity};
use crate::control::Statement;

/// The Worker's settings, from its vars and secrets.
pub struct Settings {
    /// The instance, as D1 names it.
    pub instance: String,
    /// The Artifacts namespace.
    pub namespace: String,
    /// The repository changes are for.
    pub repo: String,
    /// The lowest change number to give.
    pub floor: u32,
    /// The Access team.
    pub team: String,
    /// The Access application's audience.
    pub aud: String,
}

impl Settings {
    /// From `env`.
    ///
    /// # Errors
    ///
    /// When a var or secret is missing.
    pub fn from(env: &Env) -> Result<Self> {
        let var = |k: &str| env.var(k).map(|v| v.to_string());
        Ok(Self {
            instance: var("INSTANCE")?,
            namespace: var("NAMESPACE")?,
            repo: var("REPO")?,
            floor: var("CHANGE_FLOOR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            team: secret(env, "ACCESS_TEAM")?,
            aud: secret(env, "ACCESS_AUD")?,
        })
    }
}

/// Now, in Unix seconds.
#[must_use]
pub fn now() -> u64 {
    Date::now().as_millis() / 1000
}

/// A Worker secret's value.
///
/// # Errors
///
/// When the secret is missing or empty.
pub fn secret(env: &Env, name: &str) -> Result<String> {
    let v = env.secret(name)?.to_string();
    if v.is_empty() {
        return Err(format!("secret `{name}` is empty").into());
    }
    Ok(v)
}

/// A JSON request.
///
/// # Errors
///
/// When the request can't be built.
pub fn json_request(
    url: &str,
    method: Method,
    body: Option<&Value>,
    bearer: Option<&str>,
) -> Result<Request> {
    let headers = Headers::new();
    if let Some(t) = bearer {
        headers.set("authorization", &format!("Bearer {t}"))?;
    }
    let mut init = RequestInit::new();
    if let Some(b) = body {
        headers.set("content-type", "application/json")?;
        init.with_body(Some(JsValue::from_str(&b.to_string())));
    }
    init.with_method(method).with_headers(headers);
    Request::new_with_init(url, &init)
}

/// One Cloudflare API call with `token`: its `result`.
///
/// # Errors
///
/// When the call fails or `success` is false; never with the token.
pub async fn cloudflare(
    token: &str,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("https://api.cloudflare.com/client/v4{path}");
    let mut r = Fetch::Request(json_request(&url, method, body, Some(token))?)
        .send()
        .await?;
    let status = r.status_code();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !(200..300).contains(&status) || v["success"] == false {
        let errors = v["errors"].to_string();
        let shown = path
            .split('/')
            .map(|p| if p.len() >= 32 { "…" } else { p })
            .collect::<Vec<_>>()
            .join("/");
        return Err(format!("Cloudflare API {shown}: {status} {errors}").into());
    }
    Ok(v["result"].clone())
}

/// The one account `token` sees.
///
/// # Errors
///
/// When it sees none or more than one.
pub async fn account(token: &str) -> Result<String> {
    let v = cloudflare(token, Method::Get, "/accounts", None).await?;
    match v.as_array().map(Vec::as_slice) {
        Some([one]) => one["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "the account has no ID".into()),
        _ => Err("the Artifacts token must see exactly one account".into()),
    }
}

fn subtle() -> Result<JsValue> {
    let crypto = Reflect::get(&js_sys::global(), &"crypto".into())?;
    Ok(Reflect::get(&crypto, &"subtle".into())?)
}

fn method(on: &JsValue, name: &str) -> Result<Function> {
    Ok(Reflect::get(on, &name.into())?.dyn_into()?)
}

async fn promise(v: JsValue) -> Result<JsValue> {
    Ok(JsFuture::from(v.dyn_into::<Promise>()?).await?)
}

/// Whether `signature` over `data` verifies under RSA public key `jwk`, RS256.
///
/// # Errors
///
/// When `WebCrypto` refuses the key.
pub async fn rs256(jwk: &Value, data: &[u8], signature: &[u8]) -> Result<bool> {
    let subtle = subtle()?;
    let alg = js_sys::Object::new();
    Reflect::set(&alg, &"name".into(), &"RSASSA-PKCS1-v1_5".into())?;
    Reflect::set(&alg, &"hash".into(), &"SHA-256".into())?;
    let key_data = js_sys::JSON::parse(&jwk.to_string())?;
    let usages = Array::of1(&"verify".into());
    let import = method(&subtle, "importKey")?;
    let args = Array::of5(&"jwk".into(), &key_data, &alg, &JsValue::FALSE, &usages);
    let key = promise(import.apply(&subtle, &args)?).await?;
    let verify = method(&subtle, "verify")?;
    let args = Array::of4(
        &alg,
        &key,
        &Uint8Array::from(signature),
        &Uint8Array::from(data),
    );
    Ok(promise(verify.apply(&subtle, &args)?)
        .await?
        .as_bool()
        .unwrap_or(false))
}

/// Who `req` is from, per its Access token: decoded, its key fetched from the team's certs,
/// its signature verified, then its claims checked.
///
/// # Errors
///
/// A message for the caller; any failure refuses the request.
pub async fn identify(req: &Request, settings: &Settings) -> std::result::Result<Identity, String> {
    let jwt = req
        .headers()
        .get(access::HEADER)
        .ok()
        .flatten()
        .ok_or("no Access token")?;
    let token = access::decode(&jwt)?;
    let certs = format!("{}/cdn-cgi/access/certs", access::issuer(&settings.team));
    let url = worker::Url::parse(&certs).map_err(|e| e.to_string())?;
    let mut r = Fetch::Url(url).send().await.map_err(|e| e.to_string())?;
    let keys: Value = r.json().await.map_err(|e| e.to_string())?;
    let key = access::key(&token, &keys)?;
    if !rs256(key, token.signing_input.as_bytes(), &token.signature)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("the Access token's signature does not verify".into());
    }
    access::identity(&token, &settings.team, &settings.aud, now())
}

fn js(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::NULL,
        Value::Bool(b) => JsValue::from_bool(*b),
        Value::Number(n) => JsValue::from_f64(n.as_f64().unwrap_or_default()),
        Value::String(s) => JsValue::from_str(s),
        other => JsValue::from_str(&other.to_string()),
    }
}

/// Runs `statements` in one D1 batch.
///
/// # Errors
///
/// When D1 refuses one; none is applied then.
pub async fn d1(env: &Env, statements: &[Statement]) -> Result<()> {
    let db = env.d1("DB")?;
    let prepared = statements
        .iter()
        .map(|s| {
            db.prepare(s.sql)
                .bind(&s.params.iter().map(js).collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>>>()?;
    db.batch(prepared).await.map(|_| ())
}

/// Every row `statement` returns.
///
/// # Errors
///
/// When D1 refuses it.
pub async fn d1_rows(env: &Env, statement: &Statement) -> Result<Vec<Value>> {
    env.d1("DB")?
        .prepare(statement.sql)
        .bind(&statement.params.iter().map(js).collect::<Vec<_>>())?
        .all()
        .await?
        .results::<Value>()
}
