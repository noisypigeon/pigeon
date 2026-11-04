//! The phase 3 spike's Worker: one bearer-gated route per check; see ../README.md.

use std::time::Duration;

use serde_json::{Value, json};
use worker::{
    Container, ContainerStartupOptions, D1Database, Date, Delay, DurableObject, Env, Request,
    Response, Result, ScheduleContext, ScheduledEvent, State, durable_object, event,
};

/// The Container port the probe listens on.
const PROBE_PORT: u16 = 8080;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    let key = env.secret("SPIKE_KEY")?.to_string();
    let given = req.headers().get("authorization")?.unwrap_or_default();
    if key.len() < 32 || given != format!("Bearer {key}") {
        return Response::error("unauthorized", 401);
    }
    match req.path().as_str() {
        "/image" | "/run" | "/status" | "/uptime" | "/egress" => {
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
}

impl DurableObject for Runner {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
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
            container.start(Some(options))?;
            probe(&container, "/uptime").await?;
            startup_ms = Some(Date::now().as_millis() - started_ms);
        }
        let url = req.url()?;
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        };
        let probed_ms = Date::now().as_millis();
        let mut body = probe(&container, &path).await?;
        body["cold_start"] = json!(cold);
        body["startup_ms"] = json!(startup_ms);
        body["probe_ms"] = json!(Date::now().as_millis() - probed_ms);
        Response::from_json(&body)
    }
}

/// Asks the probe, retrying while the Container boots.
async fn probe(container: &Container, path: &str) -> Result<Value> {
    let port = container.get_tcp_port(PROBE_PORT)?;
    let mut last = String::new();
    for _ in 0..60 {
        match port.fetch(format!("http://container{path}"), None).await {
            Ok(mut r) if r.status_code() == 200 => return r.json().await,
            Ok(r) => last = format!("status {}", r.status_code()),
            Err(e) => last = e.to_string(),
        }
        Delay::from(Duration::from_millis(500)).await;
    }
    Ok(json!({ "error": format!("probe never answered: {last}") }))
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
