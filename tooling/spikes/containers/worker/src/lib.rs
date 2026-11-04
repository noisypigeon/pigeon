//! The phase 3 spike's Worker: one bearer-gated route per check; see ../README.md.

use std::time::Duration;

use serde_json::{Value, json};
use worker::wasm_bindgen::JsCast;
use worker::{
    Container, ContainerStartupOptions, D1Database, Date, Delay, DurableObject, Env, Fetcher,
    Request, Response, Result, ScheduleContext, ScheduledEvent, State, durable_object, event,
};

/// The Container port the probe listens on.
const PROBE_PORT: u16 = 8080;

/// The Container port the probe's TLS listener uses.
const TLS_PORT: u16 = 8443;

/// Worker secrets passed into the Container's environment, as PEM.
const TLS_SECRETS: [&str; 3] = ["SPIKE_TLS_CERT", "SPIKE_TLS_KEY", "SPIKE_TLS_CLIENT_CA"];

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    let key = env.secret("SPIKE_KEY")?.to_string();
    let given = req.headers().get("authorization")?.unwrap_or_default();
    if key.len() < 32 || given != format!("Bearer {key}") {
        return Response::error("unauthorized", 401);
    }
    match req.path().as_str() {
        "/image" | "/run" | "/status" | "/uptime" | "/egress" | "/tls" | "/mtls" => {
            env.durable_object("RUNNER")?
                .get_by_name("spike")?
                .fetch_with_request(req)
                .await
        }
        "/do" => {
            env.durable_object("COUNTER")?
                .get_by_name("spike")?
                .fetch_with_str("https://counter/")
                .await
        }
        "/secrets" => {
            let value = env.secret_store("SECRET")?.get().await?;
            Response::from_json(&json!({
                "present": value.is_some(),
                "length": value.map_or(0, |v| v.len()),
            }))
        }
        "/d1" => Response::from_json(&d1(&env.d1("DB")?).await?),
        "/r2" => {
            let bucket = env.bucket("BUCKET")?;
            let stamp = Date::now().as_millis().to_string();
            bucket.put("spike/probe", stamp.clone()).execute().await?;
            let back = match bucket.get("spike/probe").execute().await? {
                Some(object) => match object.body() {
                    Some(body) => body.text().await?,
                    None => String::new(),
                },
                None => String::new(),
            };
            Response::from_json(&json!({ "wrote": stamp, "read_back": back == stamp }))
        }
        _ => Response::error("unknown route", 404),
    }
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    if let Ok(db) = env.d1("DB") {
        let _ = tick(&db).await;
    }
}

async fn tick(db: &D1Database) -> Result<()> {
    db.exec("CREATE TABLE IF NOT EXISTS ticks (at INTEGER NOT NULL)")
        .await?;
    db.prepare("INSERT INTO ticks (at) VALUES (?1)")
        .bind(&[(Date::now().as_millis() as f64).into()])?
        .run()
        .await?;
    Ok(())
}

async fn d1(db: &D1Database) -> Result<Value> {
    db.exec("CREATE TABLE IF NOT EXISTS ticks (at INTEGER NOT NULL)")
        .await?;
    let row: Option<Value> = db
        .prepare("SELECT COUNT(*) AS cron_ticks, MAX(at) AS last_tick_ms FROM ticks")
        .first(None)
        .await?;
    Ok(row.unwrap_or(Value::Null))
}

/// Owns the spike's Container: starts it, then forwards checks to its probe.
#[durable_object]
pub struct Runner {
    state: State,
    env: Env,
}

impl DurableObject for Runner {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let container = self
            .state
            .container()
            .ok_or_else(|| worker::Error::RustError("no container binding".into()))?;
        let cold = !container.running();
        let mut startup_ms = None;
        if cold {
            let started_ms = Date::now().as_millis();
            let mut options = ContainerStartupOptions::new();
            options.enable_internet(true);
            for name in TLS_SECRETS {
                if let Ok(value) = self.env.secret(name) {
                    options.add_env(name, &value.to_string());
                }
            }
            container.start(Some(options))?;
            if let Err(e) = probe(&container, "/uptime", READY_TRIES).await {
                return Response::from_json(&json!({
                    "cold_start": true,
                    "ready": false,
                    "waited_ms": Date::now().as_millis() - started_ms,
                    "error": e,
                }));
            }
            startup_ms = Some(Date::now().as_millis() - started_ms);
        }
        let url = req.url()?;
        if url.path() == "/tls" || url.path() == "/mtls" {
            let mut body = tls(&self.env, &container, url.path() == "/mtls").await;
            body["cold_start"] = json!(cold);
            body["startup_ms"] = json!(startup_ms);
            return Response::from_json(&body);
        }
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        };
        let probed_ms = Date::now().as_millis();
        let mut body = match probe(&container, &path, CALL_TRIES).await {
            Ok(body) => body,
            Err(e) => json!({ "error": e }),
        };
        body["cold_start"] = json!(cold);
        body["startup_ms"] = json!(startup_ms);
        body["probe_ms"] = json!(Date::now().as_millis() - probed_ms);
        Response::from_json(&body)
    }
}

/// Half-second attempts while a cold Container boots: two minutes.
const READY_TRIES: u32 = 240;

/// Half-second attempts for a check once the Container is ready.
const CALL_TRIES: u32 = 10;

/// Asks the probe, retrying up to `tries` times; an error once they run out.
async fn probe(
    container: &Container,
    path: &str,
    tries: u32,
) -> std::result::Result<Value, String> {
    let port = container
        .get_tcp_port(PROBE_PORT)
        .map_err(|e| e.to_string())?;
    let mut last = String::new();
    for _ in 0..tries {
        match port.fetch(format!("http://container{path}"), None).await {
            Ok(mut r) if r.status_code() == 200 => {
                return r.json().await.map_err(|e| e.to_string());
            }
            Ok(r) => last = format!("status {}", r.status_code()),
            Err(e) => last = e.to_string(),
        }
        Delay::from(Duration::from_millis(500)).await;
    }
    Err(format!(
        "probe never answered {path} after {tries} tries: {last}"
    ))
}

/// Fetches the probe's TLS listener through the container binding and, for
/// `/mtls`, through the `MTLS` client-certificate binding too.
async fn tls(env: &Env, container: &Container, mtls: bool) -> Value {
    let binding = match container.get_tcp_port(TLS_PORT) {
        Ok(port) => https(&port).await,
        Err(e) => json!({ "error": e.to_string() }),
    };
    if !mtls {
        return json!({ "container_binding": binding });
    }
    let client_cert = match mtls_fetcher(env) {
        Some(fetcher) => https(&fetcher).await,
        None => json!({ "error": "no MTLS binding" }),
    };
    json!({ "container_binding": binding, "mtls_binding": client_cert })
}

/// The `[[mtls_certificates]]` binding: a fetcher whose requests carry its
/// certificate, but which reaches the internet rather than a container port.
fn mtls_fetcher(env: &Env) -> Option<Fetcher> {
    let binding = worker::js_sys::Reflect::get(env.as_ref(), &"MTLS".into()).ok()?;
    let has_fetch =
        worker::js_sys::Reflect::get(&binding, &"fetch".into()).is_ok_and(|f| f.is_function());
    has_fetch.then(|| binding.unchecked_into())
}

/// One `https://container/` fetch: the probe's answer, or why it failed.
async fn https(fetcher: &Fetcher) -> Value {
    match fetcher.fetch("https://container/", None).await {
        Ok(mut r) => {
            let status = r.status_code();
            let text = r.text().await.unwrap_or_default();
            let probe = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            json!({ "status": status, "probe": probe })
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// A Durable Object with storage, counting its own fetches.
#[durable_object]
pub struct Counter {
    state: State,
}

impl DurableObject for Counter {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, _req: Request) -> Result<Response> {
        let storage = self.state.storage();
        let count = storage.get::<u64>("count").await?.unwrap_or(0) + 1;
        storage.put("count", count).await?;
        Response::from_json(&json!({ "count": count }))
    }
}
