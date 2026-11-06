//! The `MainRun` Durable Object: `main`'s pipeline. Each push to `main` is recorded; its alarm
//! runs one deploy job at a time, of the newest push, which publishes the sites whose inputs
//! changed since the last deploy (`control::Pipeline`).

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::{self, checker, json_request, now, secret};
use crate::control::{self, Outcome, Pipeline, Report};

const PIPELINE: &str = "pipeline";
const STARTING: &str = "starting";

/// How often a running deploy is asked how it went.
const POLL: Duration = Duration::from_secs(20);

/// `main`'s pipeline; its name is `<repo>#main`.
#[durable_object]
pub struct MainRun {
    state: State,
    env: Env,
}

impl MainRun {
    async fn pipeline(&self) -> Result<Pipeline> {
        Ok(self
            .state
            .storage()
            .get(PIPELINE)
            .await?
            .unwrap_or_default())
    }

    async fn save(&self, p: &Pipeline, starting: Option<&str>) -> Result<()> {
        let storage = self.state.storage();
        storage.put(PIPELINE, p).await?;
        // `None` would reach the runtime as `undefined`, which `put` refuses.
        match starting {
            Some(sha) => storage.put(STARTING, sha).await?,
            None => {
                storage.delete(STARTING).await?;
            }
        }
        if starting.is_some() {
            storage.set_alarm(Duration::from_millis(0)).await?;
        } else if p.running.is_some() {
            storage.set_alarm(POLL).await?;
        }
        Ok(())
    }

    /// Starts the deploy of `sha` in a job Container.
    async fn start(&self, p: &Pipeline, sha: &str) -> std::result::Result<(), String> {
        let (remote, token) = platform::credentials(&self.env, false)
            .await
            .map_err(|e| e.to_string())?;
        let deploy = secret(&self.env, "DEPLOY_TOKEN").map_err(|e| e.to_string())?;
        let env = BTreeMap::from([("CLOUDFLARE_API_TOKEN".to_string(), deploy)]);
        let spec = control::deploy_spec(&remote, &token, sha, p.deployed.as_deref(), env);
        let body = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
        let req = json_request("https://checker/start", Method::Post, Some(&body), None)
            .map_err(|e| e.to_string())?;
        let mut r = checker(&self.env, &spec.id)
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

    /// The running deploy's report: `Some` when it finished, `Err(())` when its Container is
    /// gone, `None` while it runs or can't be asked.
    async fn poll(&self, sha: &str) -> Result<std::result::Result<Option<Report>, ()>> {
        let id = control::deploy_id(sha);
        let mut r = checker(&self.env, &id)?
            .fetch_with_str(&format!("https://checker/status/{id}"))
            .await?;
        Ok(match r.status_code() {
            200 => {
                let report: Report = r.json().await?;
                if report.outcome == Outcome::Running {
                    Ok(None)
                } else {
                    let key = format!("ci/{id}.log");
                    self.env
                        .bucket("CI_LOGS")?
                        .put(key, control::tail(&report.log).to_string())
                        .execute()
                        .await?;
                    Ok(Some(report))
                }
            }
            404 => Err(()),
            _ => Ok(None),
        })
    }
}

impl DurableObject for MainRun {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        match (req.method(), path.trim_matches('/')) {
            (Method::Post, "pushed") => {
                let body: Value = req.json().await?;
                let Some(sha) = body["sha"].as_str() else {
                    return Response::error("a push names its SHA", 400);
                };
                let mut p = self.pipeline().await?;
                p.pushed(sha);
                let starting = p.start();
                self.save(&p, starting.as_deref()).await?;
                Response::from_json(&p)
            }
            (Method::Get, "status") => {
                let p = self.pipeline().await?;
                Response::from_json(&json!({ "pipeline": p }))
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let mut p = self.pipeline().await?;
        let starting: Option<String> = self.state.storage().get(STARTING).await?;
        let mut next: Option<String> = None;
        if let Some(sha) = starting {
            if let Err(why) = self.start(&p, &sha).await {
                worker::console_warn!("the deploy of {sha} did not start: {why}");
                next = p.lost(now());
            }
        } else if let Some((sha, _)) = p.running.clone() {
            match self.poll(&sha).await? {
                Ok(Some(report)) => {
                    p.finished(&report, now());
                    next = p.start();
                }
                Ok(None) => {}
                Err(()) => next = p.lost(now()),
            }
        } else {
            next = p.start();
        }
        self.save(&p, next.as_deref()).await?;
        Response::ok("ok")
    }
}
