//! The `Checker` and `Runner` Durable Objects: one job's Container, started with only the
//! signing key, told what to run over the signed link, and asked how it went. They differ only
//! in their Container's size (approvals-images-state ADR §4).

use cobot::worker::{DispatchError, JobsLaunch};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::secret;

/// One check's Container, on `standard-4` (its name is the job's ID).
#[durable_object]
pub struct Checker {
    state: State,
    env: Env,
}

/// One bound, merge or deploy job's Container, on `standard-1` (its name is the job's ID).
#[durable_object]
pub struct Runner {
    state: State,
    env: Env,
}

fn failed(e: &DispatchError) -> Result<Response> {
    Response::error(e.to_string(), 502)
}

impl DurableObject for Checker {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        serve(&self.state, &self.env, req, "Checker").await
    }
}

impl DurableObject for Runner {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        serve(&self.state, &self.env, req, "Runner").await
    }
}

/// A job Durable Object's routes: `start`, `status/<id>` and `stop`.
async fn serve(state: &State, env: &Env, mut req: Request, class: &str) -> Result<Response> {
    let Some(container) = state.container() else {
        return Response::error(format!("no Container is bound to {class}"), 500);
    };
    let path = req.path();
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (req.method(), segments.as_slice()) {
        (Method::Post, ["start"]) => {
            let body = req.bytes().await?;
            let token = secret(env, cobot::worker::TOKEN_VAR)?;
            if let Err(e) = cobot::worker::start_jobs(
                state,
                &container,
                &JobsLaunch {
                    token: token.clone(),
                },
            )
            .await
            {
                return failed(&e);
            }
            // Pending I/O keeps this Durable Object, and so its Container, alive for the
            // whole job (control-plane ADR §3); without it the runtime stops both about
            // twenty seconds after the last request, and the job's report is lost.
            if let Some(held) = state.container() {
                state.wait_until(async move {
                    let _ = cobot::worker::monitor(&held).await;
                });
            }
            match cobot::worker::call(&container, &token, Method::Post, "/v0/jobs", &body).await {
                Ok((status, v)) => Ok(Response::from_json(&v)?.with_status(status)),
                Err(e) => failed(&e),
            }
        }
        (Method::Get, ["status", id]) => {
            if !container.running() {
                return Response::error("the job's Container is not running", 404);
            }
            let token = secret(env, cobot::worker::TOKEN_VAR)?;
            match cobot::worker::call(
                &container,
                &token,
                Method::Get,
                &format!("/v0/jobs/{id}"),
                &[],
            )
            .await
            {
                Ok((status, v)) => {
                    // A finished job's report is read once: then its Container is freed,
                    // as the fleet holds only `max_instances` at a time.
                    let done = v["outcome"]
                        .as_str()
                        .is_some_and(|o| o == "passed" || o == "failed");
                    if done {
                        let _ = container.destroy(Some("its job finished")).await;
                    }
                    Ok(Response::from_json(&v)?.with_status(status))
                }
                Err(DispatchError::Unreachable(_)) => {
                    Response::error("the job's Container is gone", 404)
                }
                Err(e) => failed(&e),
            }
        }
        (Method::Post, ["stop"]) => {
            if container.running() {
                container.destroy(Some("cancelled")).await?;
            }
            Response::ok("stopped")
        }
        _ => Response::error("not found", 404),
    }
}
