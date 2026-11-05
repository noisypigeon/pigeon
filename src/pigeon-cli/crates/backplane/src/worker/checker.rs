//! The `Checker` Durable Object: one job's Container, started with only the signing key, told
//! what to run over the signed link, and asked how it went.

use cobot::worker::{DispatchError, JobsLaunch};
use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object};

use super::platform::store_secret;

/// One job's Container (its name is the job's ID); reached only by `Change`.
#[durable_object]
pub struct Checker {
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

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let Some(container) = self.state.container() else {
            return Response::error("no Container is bound to Checker", 500);
        };
        let path = req.path();
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        match (req.method(), segments.as_slice()) {
            (Method::Post, ["start"]) => {
                let body = req.bytes().await?;
                let token = store_secret(&self.env, cobot::worker::TOKEN_VAR).await?;
                if let Err(e) = cobot::worker::start_jobs(
                    &container,
                    &JobsLaunch {
                        token: token.clone(),
                    },
                )
                .await
                {
                    return failed(&e);
                }
                match cobot::worker::call(&container, &token, Method::Post, "/v0/jobs", &body).await
                {
                    Ok((status, v)) => Ok(Response::from_json(&v)?.with_status(status)),
                    Err(e) => failed(&e),
                }
            }
            (Method::Get, ["status", id]) => {
                if !container.running() {
                    return Response::error("the job's Container is not running", 404);
                }
                let token = store_secret(&self.env, cobot::worker::TOKEN_VAR).await?;
                match cobot::worker::call(
                    &container,
                    &token,
                    Method::Get,
                    &format!("/v0/jobs/{id}"),
                    &[],
                )
                .await
                {
                    Ok((status, v)) => Ok(Response::from_json(&v)?.with_status(status)),
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
}
