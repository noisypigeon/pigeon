//! The control plane's Worker (control-plane ADR §1, §3, §9, §10): every request is verified
//! with Access, then served as MCP on `/mcp` or the wire form on `/v0/`; changes live in the
//! `ChangeRun` Durable Object, their jobs in `Checker` and `Runner` Containers, and Artifacts push events
//! arrive on a queue; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.
#![allow(
    missing_docs,
    reason = "`durable_object` generates undocumented wasm-bindgen items"
)]

mod approve;
mod change;
mod checker;
mod elicit;
mod events;
mod gitdoor;
mod main_run;
mod mandates;
mod merges;
mod platform;
mod platforms;
mod push;
mod tasks;
mod tickets;

use serde_json::{Value, json};
use worker::{
    DurableObject, Env, MessageBatch, MessageExt, Method, Request, Response, Result, State,
    durable_object, event,
};

pub use change::ChangeRun;
pub use checker::{Checker, Runner};
pub use main_run::MainRun;
use platform::{Settings, json_request, now};

use crate::access::{self, Identity};
use crate::changes::{Change, Event};
use crate::control::{self, Routed};
use crate::grants;
use crate::mcp::{self, Action};
use crate::runs::{Lost, Queue, Run};
use crate::wire::{self, Response as WireResponse};

/// The storage key holding an instance's [`Queue`].
const QUEUE: &str = "queue";

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    // Builds of public code, for any identity Access let through; each job checks the SHA-256
    // its Worker gave it; see .claude/decisions/2026-11-07-development-job-images-on-cloudflare.md §3.
    if req.method() == Method::Get && req.path().starts_with("/builds/") {
        return build(&env, &req.path()).await;
    }
    let settings = Settings::from(&env)?;
    let who = match platform::identify(&req, &settings).await {
        Ok(who) => who,
        Err(why) => return Response::error(format!("unauthorized: {why}"), 401),
    };
    match req.path().as_str() {
        "/healthz" => return health(&env, &settings).await,
        "/healthz/smoke" => return smoke(&env, &settings, &who).await,
        _ => {}
    }
    if let Some(refused) = narrow_token(&env, &who) {
        return refused;
    }
    if req.path().starts_with("/git/") {
        return gitdoor::serve(req, &env, &settings, &who).await;
    }
    if push::is_page(&req.path()) {
        return push::page_serve(req, &env, &settings, &who).await;
    }
    if req.path().starts_with("/approve/") || req.path() == "/approvals" {
        return approve::serve(req, &env, &settings, &who).await;
    }
    if req.path().starts_with("/platform/") {
        return platforms::page_serve(req, &env, &settings, &who).await;
    }
    if req.path().starts_with("/mandate/") {
        return mandates::page_serve(req, &env, &settings, &who).await;
    }
    match (req.method(), req.path().as_str()) {
        (Method::Post, "/mcp") => {
            let key = platform::secret(&env, "SESSION_KEY").ok();
            let session = req
                .headers()
                .get(mcp::SESSION_HEADER)?
                .zip(key.as_deref())
                .and_then(|(id, k)| mcp::session(&id, k.as_bytes()))
                .unwrap_or_default();
            let message: Value = match req.json().await {
                Ok(v) => v,
                Err(_) => {
                    return Response::from_json(&mcp::error(
                        &Value::Null,
                        mcp::code::INVALID_REQUEST,
                        "not JSON",
                    ));
                }
            };
            match mcp::parse(&message, crate::VERSION) {
                Action::Reply(v) => Response::from_json(&v),
                Action::Initialize { reply, session } => {
                    let mut r = Response::from_json(&reply)?;
                    if let Some(k) = key {
                        r.headers_mut().set(
                            mcp::SESSION_HEADER,
                            &mcp::session_id(&session, k.as_bytes()),
                        )?;
                    }
                    Ok(r)
                }
                Action::Accept => Ok(Response::empty()?.with_status(202)),
                Action::Answer { id, result } => elicit::answer(&env, &id, &result).await,
                Action::Call { id, request } => {
                    if request.operation == "approval.wait" {
                        return wait(&env, &session, &id, &request).await;
                    }
                    if !who.human && grants::OPERATIONS.contains(&request.operation.as_str()) {
                        return offer(&env, &settings, &id, &request).await;
                    }
                    let outcome = run(&env, &settings, &who, &request).await;
                    Response::from_json(&mcp::answer(&id, outcome))
                }
            }
        }
        (Method::Get, "/v0/operations") => Response::from_json(&wire::catalog()),
        (Method::Get, "/v0/events") => {
            let url = req.url()?;
            let args: serde_json::Map<String, Value> = url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect();
            Response::from_json(&events::wait(&env, &settings, &args, who.human).await?)
        }
        (Method::Post, "/v0/run") => {
            let request: wire::Request = req.json().await?;
            match run(&env, &settings, &who, &request).await {
                Ok(r) => Response::from_json(&r),
                Err(e) => Response::error(e.to_string(), 400),
            }
        }
        _ => Response::error("not found", 404),
    }
}

/// The change number of `request`'s `number` argument.
fn number_arg(request: &wire::Request) -> Option<u32> {
    request.args.get("number").and_then(|v| match v {
        Value::String(s) => s.parse().ok(),
        other => other.as_u64().and_then(|n| u32::try_from(n).ok()),
    })
}

/// A service token's call of an owner's operation: pending, with a link to the approval page,
/// at once; `approval.wait` shows the card on a client that shows forms; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §4.
async fn offer(
    env: &Env,
    settings: &Settings,
    id: &Value,
    request: &wire::Request,
) -> Result<Response> {
    let refused =
        |why: String| Response::from_json(&mcp::answer(id, Err(crate::Error::Invalid(vec![why]))));
    let Some(number) = number_arg(request) else {
        return refused("`number` must be a change number".into());
    };
    if let Err(why) = tickets::guard(env, settings, request).await? {
        return refused(why);
    }
    if let Some(r) = under_mandate(env, settings, number, request).await? {
        return Response::from_json(&mcp::answer(id, r));
    }
    if let Some(r) = under_policy(env, settings, number, request).await? {
        return Response::from_json(&mcp::answer(id, r));
    }
    if let Some(r) = under_change_grant(env, settings, number, request).await? {
        return Response::from_json(&mcp::answer(id, r));
    }
    let host = env
        .var(cobot::ship_wire::HOST_VAR)
        .map(|v| v.to_string())
        .unwrap_or_default();
    match approve::pending(env, settings, &host, number, &request.operation).await {
        Ok((link, expires)) => {
            let op = &request.operation;
            let said = format!("`{op}` on !{number} waits for the owner");
            let _ = events::append(
                env,
                settings,
                "approval.pending",
                Some(number),
                None,
                &said,
                false,
            )
            .await;
            push::notify(
                env,
                settings,
                "Approval waiting",
                &said,
                link.strip_prefix(&format!("https://{host}"))
                    .unwrap_or(&link),
            )
            .await;
            let linked = format!("{said}: {link}");
            let _ = events::append(
                env,
                settings,
                "approval.pending",
                Some(number),
                None,
                &linked,
                true,
            )
            .await;
            Response::from_json(&mcp::answer(
                id,
                Ok(wire::Response {
                    operation: request.operation.clone(),
                    ok: false,
                    result: json!({
                        "pending": { "operation": request.operation, "number": number, "link": link, "expires": expires }
                    }),
                    violations: vec![format!(
                        "waiting for the owner: `{}` on change {number} is approved at {link} with the authenticator code, or in the card `approval_wait` shows",
                        request.operation
                    )],
                }),
            ))
        }
        Err(why) => refused(why),
    }
}

/// Runs `request` on change `number` as its mandate, when the mandate approves it; see
/// `.claude/decisions/2026-11-08-security-tiered-approvals-task-mandates.md` §3.
async fn under_mandate(
    env: &Env,
    s: &Settings,
    number: u32,
    request: &wire::Request,
) -> Result<Option<std::result::Result<WireResponse, crate::Error>>> {
    let (status, v) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
    if status != 200 {
        return Ok(None);
    }
    let Ok(change) = serde_json::from_value::<Change>(v["change"].clone()) else {
        return Ok(None);
    };
    let Some(m) = mandates::approves(env, s, &change, &request.operation).await? else {
        return Ok(None);
    };
    let who = Identity {
        subject: format!("mandate:{m}"),
        human: true,
    };
    Ok(Some(run(env, s, &who, request).await))
}

/// Runs `request` on change `number` as `policy:<rule>`, when its head's tier lets the control
/// plane approve it; see `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §1.
async fn under_policy(
    env: &Env,
    s: &Settings,
    number: u32,
    request: &wire::Request,
) -> Result<Option<std::result::Result<WireResponse, crate::Error>>> {
    let (status, v) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
    if status != 200 {
        return Ok(None);
    }
    let Ok(change) = serde_json::from_value::<Change>(v["change"].clone()) else {
        return Ok(None);
    };
    let Some(rule) = crate::approvals::by_policy(&change, &request.operation) else {
        return Ok(None);
    };
    let who = Identity {
        subject: format!("policy:{rule}"),
        human: true,
    };
    Ok(Some(run(env, s, &who, request).await))
}

/// Runs `request` on change `number` as `grant:<owner>`, when the owner's one proof for it
/// still covers the operation; see `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md` §3.
async fn under_change_grant(
    env: &Env,
    s: &Settings,
    number: u32,
    request: &wire::Request,
) -> Result<Option<std::result::Result<WireResponse, crate::Error>>> {
    let (status, v) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
    if status != 200 {
        return Ok(None);
    }
    let Ok(change) = serde_json::from_value::<Change>(v["change"].clone()) else {
        return Ok(None);
    };
    let Some(by) = crate::approvals::by_grant(&change, &request.operation, now()) else {
        return Ok(None);
    };
    let who = Identity {
        subject: format!("grant:{by}"),
        human: true,
    };
    Ok(Some(run(env, s, &who, request).await))
}

/// `approval.wait`: the owner's code card for `operation` on change `number`, when the client
/// shows forms.
async fn wait(
    env: &Env,
    session: &mcp::Session,
    id: &Value,
    request: &wire::Request,
) -> Result<Response> {
    let refused =
        |why: String| Response::from_json(&mcp::answer(id, Err(crate::Error::Invalid(vec![why]))));
    let Some(number) = number_arg(request) else {
        return refused("`number` must be a change number".into());
    };
    let operation = request
        .args
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !grants::OPERATIONS.contains(&operation.as_str()) {
        return refused(format!("`{operation}` isn't approved by the owner"));
    }
    if !session.form_elicitation {
        return refused(
            "this client doesn't show forms; the owner approves at the page's link".into(),
        );
    }
    match elicit::card(env.clone(), id.clone(), number, operation).await {
        Ok(r) => Ok(r),
        Err(why) => refused(why),
    }
}

fn change_stub(env: &Env, repo: &str, number: u32) -> Result<worker::Stub> {
    env.durable_object("CHANGE_RUN")?
        .id_from_name(&format!("{repo}#{number}"))?
        .get_stub()
}

/// Sends `body` to change `number`'s Durable Object at `path`: its status and JSON answer.
async fn to_change(
    env: &Env,
    repo: &str,
    number: u32,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value)> {
    let req = json_request(&format!("https://change{path}"), method, body, None)?;
    let mut r = platform::call(env, &change_stub(env, repo, number)?, req).await?;
    let status = r.status_code();
    let text = r.text().await?;
    Ok((
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    ))
}

fn respond(operation: &str, status: u16, result: Value) -> WireResponse {
    let ok = (200..300).contains(&status);
    let violations = if ok {
        result["change"]["violations"]
            .as_array()
            .or_else(|| result["violations"].as_array())
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![
            result
                .as_str()
                .map_or_else(|| result.to_string(), str::to_string),
        ]
    };
    WireResponse {
        operation: operation.to_string(),
        ok: ok && violations.is_empty(),
        result,
        violations,
    }
}

/// Runs one operation as `who`: validated, authorized, routed and audited.
async fn run(
    env: &Env,
    s: &Settings,
    who: &Identity,
    request: &wire::Request,
) -> std::result::Result<WireResponse, crate::Error> {
    let op = wire::validate(request)?;
    access::authorize(op, who).map_err(|m| crate::Error::Invalid(vec![m]))?;
    if !who.human && op.effect != wire::Effect::Read {
        tickets::guard(env, s, request)
            .await
            .map_err(|e| crate::Error::Invalid(vec![e.to_string()]))?
            .map_err(|m| crate::Error::Invalid(vec![m]))?;
    }
    if !op.control_plane() {
        return Err(crate::Error::Invalid(vec![format!(
            "`{}` runs in an instance's Container, which the control plane doesn't dispatch yet",
            op.name
        )]));
    }
    let mut request = request.clone();
    if op.name == "change.approve" && !request.args.contains_key("sha") {
        let number = control::route(
            &wire::Request {
                operation: "change.status".into(),
                args: request.args.clone(),
            },
            &who.subject,
            now(),
        )
        .ok()
        .and_then(|r| match r {
            Routed::Status { number } => Some(number),
            _ => None,
        })
        .ok_or_else(|| crate::Error::Invalid(vec!["`number` must be a change number".into()]))?;
        let (_, v) = to_change(env, &s.repo, number, Method::Get, "/status", None)
            .await
            .map_err(|e| crate::Error::Invalid(vec![e.to_string()]))?;
        let head = v["change"]["head"]
            .as_str()
            .ok_or_else(|| crate::Error::Invalid(vec![format!("change {number} has no head")]))?;
        request.args.insert("sha".into(), Value::from(head));
    }
    let request = &request;
    let routed =
        control::route(request, &who.subject, now()).map_err(|m| crate::Error::Invalid(vec![m]))?;
    let outcome = dispatch(env, s, who, op.name, routed).await;
    let (target, result) = match &outcome {
        Ok((t, r)) => (t.clone(), if r.ok { "ok" } else { "refused" }),
        Err(_) => (None, "error"),
    };
    let audit = control::audit(
        now(),
        &who.subject,
        op.name,
        &s.instance,
        target.as_deref(),
        result,
    );
    if let Err(e) = platform::d1(env, &[control::instance_row(&s.instance, now()), audit]).await {
        worker::console_error!("audit: {e}");
    }
    outcome
        .map(|(_, r)| r)
        .map_err(|e| crate::Error::Invalid(vec![e.to_string()]))
}

async fn dispatch(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    routed: Routed,
) -> Result<(Option<String>, WireResponse)> {
    match routed {
        Routed::Propose {
            slug,
            title,
            plan,
            scope,
            mandate,
            session,
            affects,
        } => {
            let mut change = Change::propose(0, &slug, &title, &plan, scope, &who.subject);
            change.mandate = mandate;
            change.session = session;
            change.affects = affects;
            propose(env, s, operation, change).await
        }
        Routed::Event {
            number,
            event: Event::Merge { by },
        } => {
            let (status, v) = merges::ask(env, s, number, &by).await?;
            Ok((Some(number.to_string()), respond(operation, status, v)))
        }
        Routed::Event { number, event } => {
            let body = serde_json::to_value(&event)?;
            let (status, v) =
                to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await?;
            Ok((
                Some(number.to_string()),
                respond(operation, status, json!({ "change": v })),
            ))
        }
        Routed::Status { number } => {
            let (status, v) = to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
            Ok((Some(number.to_string()), respond(operation, status, v)))
        }
        Routed::DeployRetry => {
            let req = json_request("https://main/retry", Method::Post, None, None)?;
            let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
            let v: Value = r.json().await?;
            Ok((None, respond(operation, r.status_code(), v)))
        }
        Routed::DeployStatus => {
            let req = json_request("https://main/status", Method::Get, None, None)?;
            let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
            let mut v: Value = r.json().await?;
            v["pipeline"]["last_log"] = json!(last_log(env, &v).await?);
            Ok((None, respond(operation, r.status_code(), v)))
        }
        Routed::GitTokens => {
            let api = platform::secret(env, "ARTIFACTS_TOKEN")?;
            let account = platform::account(&api).await?;
            let path = format!(
                "/accounts/{account}/artifacts/namespaces/{}/repos/{}/tokens?state=all&per_page=100",
                s.namespace, s.repo
            );
            let v = platform::cloudflare(&api, Method::Get, &path, None).await?;
            let tokens = control::token_rows(&v);
            Ok((None, respond(operation, 200, json!({ "tokens": tokens }))))
        }
        Routed::List { state } => {
            let rows =
                platform::d1_rows(env, &control::list(&s.instance, &s.repo, state.as_deref()))
                    .await?;
            Ok((None, respond(operation, 200, json!({ "changes": rows }))))
        }
        Routed::SessionEnd { args } => Ok(answered(operation, tickets::end(env, s, &args).await?)),
        Routed::SessionStart { args } => {
            Ok(answered(operation, tickets::start(env, s, &args).await?))
        }
        Routed::Digest => Ok((
            None,
            respond(operation, 200, push::show(env, s, who.human).await?),
        )),
        Routed::Events { args } => {
            let v = events::wait(env, s, &args, who.human).await?;
            Ok((None, respond(operation, 200, v)))
        }
        Routed::Task {
            operation: op,
            args,
        } => Ok(answered(operation, tasks::run(env, s, &op, &args).await?)),
        Routed::Mandate {
            operation: op,
            args,
        } => {
            let (status, v) = mandates::run(env, s, who, &op, &args).await?;
            Ok((None, respond(operation, status, v)))
        }
        Routed::Platform {
            operation: op,
            args,
        } => {
            let (status, v) = platforms::run(env, s, &op, &args).await?;
            Ok((None, respond(operation, status, v)))
        }
    }
}

/// Numbers `change` and stores it, answering the open changes it overlaps.
async fn propose(
    env: &Env,
    s: &Settings,
    operation: &str,
    mut change: Change,
) -> Result<(Option<String>, WireResponse)> {
    let open =
        control::open_rows(&platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?);
    if change.proposer != crate::reconcile::PROPOSER
        && let Err(why) = crate::sessions::limit(&open, change.session.as_deref())
    {
        return Ok((None, respond(operation, 409, json!(why))));
    }
    let overlaps = crate::sessions::overlaps(&change.scope, &change.affects, &open);
    for _ in 0..3 {
        let rows =
            platform::d1_rows(env, &control::next_number(&s.instance, &s.repo, s.floor)).await?;
        change.number = rows
            .first()
            .and_then(|r| r["n"].as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(s.floor);
        let number = change.number;
        let body = serde_json::to_value(&change)?;
        let (status, v) =
            to_change(env, &s.repo, number, Method::Post, "/propose", Some(&body)).await?;
        if status != 409 {
            let result = json!({
                "number": number,
                "branch": change.branch(),
                "change": v,
                "overlaps": overlaps,
            });
            return Ok((Some(number.to_string()), respond(operation, status, result)));
        }
    }
    Ok((
        None,
        respond(
            operation,
            409,
            json!("no change number was free; try again"),
        ),
    ))
}

/// An operation's answer from a `(status, value)` pair.
fn answered(operation: &str, (status, v): (u16, Value)) -> (Option<String>, WireResponse) {
    (None, respond(operation, status, v))
}

/// `/healthz`: the version serving, and whether D1 and a Durable Object answer; see
/// .claude/decisions/2026-11-07-development-control-plane-deploys-itself.md §7.
async fn health(env: &Env, s: &Settings) -> Result<Response> {
    let version = env
        .get_binding::<worker::WorkerVersionMetadata>("CF_VERSION_METADATA")
        .map(|v| v.id())
        .unwrap_or_default();
    let mine = platform::version(env);
    let d1 = match env.d1("DB") {
        Ok(db) => db.prepare("SELECT 1").all().await.is_ok(),
        Err(_) => false,
    };
    let ask = |url: &'static str| async move {
        let req = json_request(url, Method::Get, None, None).ok()?;
        let stub = if url.starts_with("https://main") {
            main_stub(env, &s.repo).ok()?
        } else {
            env.durable_object("INSTANCE")
                .ok()?
                .id_from_name(&s.instance)
                .ok()?
                .get_stub()
                .ok()?
        };
        let mut r = platform::call(env, &stub, req).await.ok()?;
        match r.status_code() {
            200 => r.text().await.ok(),
            404 => Some(String::new()),
            _ => None,
        }
    };
    let main = ask("https://main/version").await;
    let instance = ask("https://instance/version").await;
    let object = instance.is_some();
    let current = crate::versions::object_current(instance.as_deref(), &mine);
    let ok = d1 && object && current && !version.is_empty();
    Ok(Response::from_json(&json!({
        "ok": ok,
        "version": version,
        "instance": s.instance,
        "d1": d1,
        "durable_object": object,
        "objects": { "main": main, "instance": instance, "expected": mine },
    }))?
    .with_status(if ok { 200 } else { 503 }))
}

/// The newest deploy's log, from where its job stored it.
async fn last_log(env: &Env, status: &Value) -> Result<Option<String>> {
    let Some(sha) = status["pipeline"]["history"][0]["sha"].as_str() else {
        return Ok(None);
    };
    let key = format!("ci/{}.log", control::deploy_id(sha));
    match env.bucket("CI_LOGS")?.get(key).execute().await? {
        Some(o) => match o.body() {
            Some(b) => Ok(Some(b.text().await?)),
            None => Ok(None),
        },
        None => Ok(None),
    }
}

/// The refusal for a token that reaches only `/healthz` and `/builds/`, past those.
fn narrow_token(env: &Env, who: &Identity) -> Option<Result<Response>> {
    let is = |var: &str| platform::secret(env, var).is_ok_and(|id| id == who.subject);
    if is(cobot::ship_wire::ACCESS_ID_VAR) {
        return Some(Response::error(
            "the deployer's token reads /healthz and /healthz/smoke only",
            403,
        ));
    }
    if is(cobot::ship_wire::BUILDS_ACCESS_ID_VAR) {
        return Some(Response::error("the builds token reads /builds/ only", 403));
    }
    None
}

/// `/healthz/smoke`: this version's own code answering, with no write: D1, both objects at any
/// version, MCP `tools/list`, the newest change's status and `deploy.status`; see
/// .claude/decisions/2026-11-08-runtime-blue-green-self-standup.md, step 2's spike.
async fn smoke(env: &Env, s: &Settings, who: &Identity) -> Result<Response> {
    let version = env
        .get_binding::<worker::WorkerVersionMetadata>("CF_VERSION_METADATA")
        .map(|v| v.id())
        .unwrap_or_default();
    let d1 = match env.d1("DB") {
        Ok(db) => db.prepare("SELECT 1").all().await.is_ok(),
        Err(_) => false,
    };
    let answers = |url: &'static str| async move {
        let req = json_request(url, Method::Get, None, None).ok()?;
        let stub = if url.starts_with("https://main") {
            main_stub(env, &s.repo).ok()?
        } else {
            env.durable_object("INSTANCE")
                .ok()?
                .id_from_name(&s.instance)
                .ok()?
                .get_stub()
                .ok()?
        };
        let r = platform::call(env, &stub, req).await.ok()?;
        (r.status_code() < 500).then_some(())
    };
    let instance = answers("https://instance/version").await.is_some();
    let main = answers("https://main/version").await.is_some();
    let tools = match mcp::parse(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        crate::VERSION,
    ) {
        Action::Reply(v) => v["result"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        _ => false,
    };
    let read = |operation: &str, args: Value| wire::Request {
        operation: operation.into(),
        args: serde_json::from_value(args).unwrap_or_default(),
    };
    let listed = run(env, s, who, &read("change.list", json!({}))).await;
    let newest = listed
        .as_ref()
        .ok()
        .and_then(|r| r.result["changes"][0]["number"].as_u64());
    let change = match newest {
        Some(n) => run(
            env,
            s,
            who,
            &read("change.status", json!({ "number": n.to_string() })),
        )
        .await
        .is_ok_and(|r| r.ok),
        None => listed.is_ok_and(|r| r.ok),
    };
    let deploy = run(env, s, who, &read("deploy.status", json!({})))
        .await
        .is_ok_and(|r| r.ok);
    let probes = json!({
        "d1": d1, "instance": instance, "main": main, "tools": tools,
        "change_status": change, "deploy_status": deploy,
    });
    let ok = !version.is_empty()
        && probes
            .as_object()
            .is_some_and(|m| m.values().all(|v| v == true));
    Ok(
        Response::from_json(&json!({ "ok": ok, "version": version, "probes": probes }))?
            .with_status(if ok { 200 } else { 503 }),
    )
}

/// `GET /builds/<sha>/pigeon`: `main`'s `pigeon` built at that commit, from R2.
async fn build(env: &Env, path: &str) -> Result<Response> {
    let Some(sha) = cobot::ship_wire::build_path_sha(path) else {
        return Response::error("not found", 404);
    };
    let Some(object) = env
        .bucket("CI_LOGS")?
        .get(cobot::ship_wire::build_key(sha))
        .execute()
        .await?
    else {
        return Response::error("no build of that commit", 404);
    };
    let Some(body) = object.body() else {
        return Response::error("no build of that commit", 404);
    };
    // Streamed: a build is tens of MB, too much to hold in the Worker's memory.
    let mut r = Response::from_stream(body.stream()?)?;
    let h = r.headers_mut();
    h.set("content-type", "application/octet-stream")?;
    h.set("cache-control", "public, max-age=31536000, immutable")?;
    Ok(r)
}

/// `main`'s pipeline's newest build, as job Containers fetch it; none when there is no build
/// yet, no public host, or no builds token to fetch it with.
pub(super) async fn latest_build(env: &Env) -> Option<cobot::worker::JobsBuild> {
    let host = env.var(cobot::ship_wire::HOST_VAR).ok()?.to_string();
    let access = (
        platform::secret(env, cobot::ship_wire::BUILDS_ACCESS_ID_VAR).ok()?,
        platform::secret(env, cobot::ship_wire::BUILDS_ACCESS_SECRET_VAR).ok()?,
    );
    let repo = env.var("REPO").ok()?.to_string();
    let mut r = main_stub(env, &repo)
        .ok()?
        .fetch_with_str("https://main/status")
        .await
        .ok()?;
    let v: Value = r.json().await.ok()?;
    let b: control::Build = serde_json::from_value(v["pipeline"]["latest_build"].clone()).ok()?;
    Some(cobot::worker::JobsBuild {
        url: format!("https://{host}/builds/{}/pigeon", b.sha),
        sha256: b.sha256,
        access,
    })
}

fn main_stub(env: &Env, repo: &str) -> Result<worker::Stub> {
    env.durable_object("MAIN_RUN")?
        .id_from_name(&format!("{repo}#main"))?
        .get_stub()
}

/// The hourly digest; see
/// `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §4.
#[event(scheduled)]
async fn scheduled(_event: worker::ScheduledEvent, env: Env, _ctx: worker::ScheduleContext) {
    if let Err(e) = push::hourly(&env).await {
        worker::console_error!("digest: {e}");
    }
}

/// Artifacts push events: a push to a change's branch is that change's `Pushed`; a push to
/// `main` goes to its pipeline.
#[event(queue)]
async fn queue(batch: MessageBatch<Value>, env: Env, _ctx: worker::Context) -> Result<()> {
    let s = Settings::from(&env)?;
    for message in batch.messages()? {
        let Some(push) = control::push(message.body()) else {
            message.ack();
            continue;
        };
        if push.repo.as_deref().is_some_and(|r| r != s.repo) {
            message.ack();
            continue;
        }
        let number = match push.target {
            control::Target::Main => {
                let body = json!({ "sha": push.sha });
                let req = json_request("https://main/pushed", Method::Post, Some(&body), None)?;
                match platform::call(&env, &main_stub(&env, &s.repo)?, req).await {
                    Ok(r) if (200..300).contains(&r.status_code()) => message.ack(),
                    _ => message.retry(),
                }
                continue;
            }
            control::Target::Change(n) => n,
        };
        let body = serde_json::to_value(Event::Pushed {
            sha: push.sha.clone(),
        })?;
        match to_change(&env, &s.repo, number, Method::Post, "/event", Some(&body)).await {
            Ok((status, v)) if (200..300).contains(&status) || status == 409 || status == 404 => {
                if status != 200 {
                    worker::console_warn!("push to change {number} refused: {v}");
                }
                message.ack();
            }
            _ => message.retry(),
        }
    }
    Ok(())
}

/// One instance's runs, one at a time (ADR §3); reached only by the Worker.
#[durable_object]
pub struct Instance {
    state: State,
    env: Env,
}

impl DurableObject for Instance {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        if path == "/version" {
            return Response::ok(platform::version(&self.env));
        }
        if path.starts_with("/elicit/") || path.starts_with("/approvals/") || path == "/code/verify"
        {
            let body: Value = req.json().await?;
            return elicit::serve(&self.state, &self.env, &path, body).await;
        }
        if path.starts_with("/mandates/") {
            let body: Value = req.json().await?;
            return mandates::serve(&self.state, &path, body).await;
        }
        if path.starts_with("/tickets/") {
            let body: Value = req.json().await?;
            return tickets::serve(&self.state, &path, &body).await;
        }
        if path.starts_with("/push/") || path.starts_with("/digest/") {
            let body: Value = req.json().await?;
            return push::serve(&self.state, &path, &body).await;
        }
        if path.starts_with("/events/") {
            let body: Value = req.json().await?;
            return events::serve(&self.state, &path, &body).await;
        }
        if path.starts_with("/tasks/") {
            let body: Value = req.json().await?;
            return tasks::serve(&self.state, &path, &body).await;
        }
        if path.starts_with("/merges/") {
            let body: Value = req.json().await?;
            return merges::serve(&self.state, &path, &body).await;
        }
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        let method = req.method();
        // Only storage is awaited between the queue's read and write (ADR §3).
        let submitted: Option<Run> = match (&method, segments.as_slice()) {
            (Method::Post, ["runs"]) => Some(req.json().await?),
            _ => None,
        };
        let storage = self.state.storage();
        let mut queue: Queue = storage.get(QUEUE).await?.unwrap_or_default();
        let answer = match (method, segments.as_slice(), submitted) {
            (Method::Get, ["runs"], _) => return Response::from_json(&queue),
            (Method::Post, ["runs"], Some(run)) => match queue.enqueue(run) {
                Ok(()) => Response::from_json(&queue),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", "start"], _) => Response::from_json(&queue.start().cloned()),
            (Method::Post, ["runs", id, "finish"], _) => match queue.finish(id) {
                Ok(run) => Response::from_json(&run),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", id, "lost"], _) => match queue.lost(id) {
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
