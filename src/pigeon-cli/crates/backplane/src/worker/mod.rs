//! The control plane's Worker: its fetch entry and the `Instance` Durable Object
//! (control-plane ADR §1, §3). Every route is refused until Access verification lands (§9).
#![allow(
    missing_docs,
    reason = "`durable_object` generates undocumented wasm-bindgen items"
)]

use worker::{DurableObject, Env, Method, Request, Response, Result, State, durable_object, event};

use crate::runs::{Lost, Queue, Run};

/// The storage key holding an instance's [`Queue`].
const QUEUE: &str = "queue";

#[event(fetch)]
async fn fetch(_req: Request, _env: Env, _ctx: worker::Context) -> Result<Response> {
    Response::error("unauthorized", 401)
}

/// One instance's runs, one at a time (ADR §3); reached only by the Worker.
#[durable_object]
pub struct Instance {
    state: State,
}

impl DurableObject for Instance {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let storage = self.state.storage();
        let mut queue: Queue = storage.get(QUEUE).await?.unwrap_or_default();
        let path = req.path();
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        let answer = match (req.method(), segments.as_slice()) {
            (Method::Get, ["runs"]) => return Response::from_json(&queue),
            (Method::Post, ["runs"]) => {
                let run: Run = req.json().await?;
                match queue.enqueue(run) {
                    Ok(()) => Response::from_json(&queue),
                    Err(e) => return Response::error(e.to_string(), 409),
                }
            }
            (Method::Post, ["runs", "start"]) => Response::from_json(&queue.start().cloned()),
            (Method::Post, ["runs", id, "finish"]) => match queue.finish(id) {
                Ok(run) => Response::from_json(&run),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", id, "lost"]) => match queue.lost(id) {
                Ok(Lost::Retry(run)) => Response::from_json(&serde_json::json!({ "retry": run })),
                Ok(Lost::Failed(run)) => Response::from_json(&serde_json::json!({ "failed": run })),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            _ => return Response::error("not found", 404),
        };
        storage.put(QUEUE, &queue).await?;
        answer
    }
}
