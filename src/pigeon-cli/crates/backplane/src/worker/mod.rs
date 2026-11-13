//! The control plane's Worker (control-plane ADR §1, §3, §9, §10): every request is verified
//! with Access, then served as MCP on `/mcp` or the wire form on `/v0/`; changes live in the
//! `ChangeRun` Durable Object, their jobs in `Checker` and `Runner` Containers, and Artifacts push events
//! arrive on a queue; see `.claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md`.
#![allow(
    missing_docs,
    reason = "`durable_object` generates undocumented wasm-bindgen items"
)]

mod api;
mod apns;
mod app;
mod apple;
mod approve;
mod buildkite;
mod change;
mod checker;
mod ci_window;
mod devices;
mod docs;
mod elicit;
mod events;
mod gitdoor;
mod github;
mod insights;
mod main_run;
mod mandates;
mod merges;
mod passkeys;
mod platform;
mod platforms;
mod push;
mod stages;
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
async fn fetch(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    // Builds of public code, for any identity Access let through; each job checks the SHA-256
    // its Worker gave it; see .claude/decisions/2026-11-07-development-job-images-on-cloudflare.md §3.
    if req.method() == Method::Get && req.path().starts_with("/builds/") {
        return build(&env, &req.path()).await;
    }
    // The one public path: Access lets it through, and the route authenticates its caller with
    // a Buildkite OIDC token; see .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md §5.
    if req.path() == crate::apple::ROUTE {
        return if req.method() == Method::Post {
            apple::release(req, &env).await
        } else {
            Response::error("method not allowed", 405)
        };
    }
    if req.path() == format!("{}/archive", crate::apple::ROUTE) {
        return if req.method() == Method::Get {
            apple::archive(req, &env).await
        } else {
            Response::error("method not allowed", 405)
        };
    }
    if req.path().starts_with(&format!("{}/", crate::apple::ROUTE)) {
        return Response::error("not found", 404);
    }
    // The app's API and Apple's association file: Access lets them through, and `/v0/app/*`
    // authenticates each call by its device's signature; see
    // .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §4.
    if req.path() == devices::ASSOCIATION || req.path().starts_with(devices::API) {
        return devices::public(req, &env)
            .await
            .unwrap_or_else(|| Response::error("not found", 404));
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
    let (method, path) = (req.method(), req.path());
    match crate::routes::find(SURFACES, &method, &path) {
        Some(route) => (route.serve)(req, &env, &settings, &who).await,
        None => Response::error("not found", 404),
    }
}

/// What serves a route: the request, with the Worker's bindings, settings and caller.
pub(super) type Serve = for<'a> fn(
    Request,
    &'a Env,
    &'a Settings,
    &'a Identity,
) -> futures_util::future::LocalBoxFuture<'a, Result<Response>>;

/// A route of one of the Worker's surfaces.
pub(super) type Route = crate::routes::Route<Method, Serve>;

/// The router: each surface's routes, which it declares beside its handlers; see
/// `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §4.
const SURFACES: &[&[Route]] = &[
    api::ROUTES,
    gitdoor::ROUTES,
    push::ROUTES,
    approve::ROUTES,
    passkeys::ROUTES,
    platforms::ROUTES,
    mandates::ROUTES,
    ci_window::ROUTES,
    docs::ROUTES,
    devices::ROUTES,
];

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
    let answer = ask(env, settings, number, request).await?;
    Response::from_json(&mcp::answer(id, answer))
}

/// Asks for `request` on change `number` as someone who can't approve it: run under a mandate,
/// an Auto window, a policy or the change's grant when one approves it, else pending, with the
/// approval page's link, an event and a push to the owner. The control plane asks this way too.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn ask(
    env: &Env,
    settings: &Settings,
    number: u32,
    request: &wire::Request,
) -> Result<std::result::Result<WireResponse, crate::Error>> {
    if let Some(r) = under_mandate(env, settings, number, request).await? {
        return Ok(r);
    }
    if let Some(r) = under_window(env, settings, number, request).await? {
        return Ok(r);
    }
    if let Some(r) = under_policy(env, settings, number, request).await? {
        return Ok(r);
    }
    if let Some(r) = under_change_grant(env, settings, number, request).await? {
        return Ok(r);
    }
    let host = env
        .var(crate::cobot::ship_wire::HOST_VAR)
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
            Ok(Ok(wire::Response {
                operation: request.operation.clone(),
                ok: false,
                result: json!({
                    "pending": { "operation": request.operation, "number": number, "link": link, "expires": expires }
                }),
                violations: vec![format!(
                    "waiting for the owner: `{}` on change {number} is approved at {link} with the authenticator code, or in the card `approval_wait` shows",
                    request.operation
                )],
            }))
        }
        Err(why) => Ok(Err(crate::Error::Invalid(vec![why]))),
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

/// Runs `request` on change `number` as `window:<id>`, when the owner's Auto window allows it;
/// see `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md` §5.
async fn under_window(
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
    let Some(id) = mandates::window_approves(env, s, &change, &request.operation).await? else {
        return Ok(None);
    };
    let who = Identity {
        subject: format!("window:{id}"),
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
    let retargeted = control::retarget(request);
    let request = &retargeted;
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
    let hold = request
        .args
        .get("hold")
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));
    let outcome = dispatch(env, s, who, op.name, routed, hold).await;
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

/// `change.approve` on change `number`, then, unless `hold`, its merge asked for as the
/// approver; see the 2026-11-12 amendment to
/// `.claude/decisions/2026-11-09-security-approvals-once-passkeys.md`.
async fn approve(
    env: &Env,
    s: &Settings,
    operation: &str,
    number: u32,
    event: &Event,
    hold: bool,
) -> Result<(Option<String>, WireResponse)> {
    let body = serde_json::to_value(event)?;
    let (status, v) = to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await?;
    let mut result = json!({ "change": v });
    if let Event::Approve { by, .. } = event
        && status == 200
        && !hold
        && v["state"] == "approved"
    {
        let (merge_status, merge) = merges::ask(env, s, number, by).await?;
        result["merge"] = json!({ "status": merge_status, "result": merge });
    }
    Ok((Some(number.to_string()), respond(operation, status, result)))
}

/// The owner's approved break-glass on change `number`; see
/// `.claude/decisions/2026-11-10-development-code-checks-github-actions.md` §5.
async fn break_glass(
    env: &Env,
    s: &Settings,
    operation: &str,
    number: u32,
) -> Result<(Option<String>, WireResponse)> {
    let (status, v) = to_change(env, &s.repo, number, Method::Post, "/cloudflare", None).await?;
    Ok((
        Some(number.to_string()),
        respond(operation, status, json!({ "change": v })),
    ))
}

#[expect(clippy::too_many_lines, reason = "one arm per control-plane operation")]
async fn dispatch(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    routed: Routed,
    hold: bool,
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
            full,
            resumes,
            after,
        } => {
            let built = match after {
                Some(n) => match built_on(env, s, n).await? {
                    Ok(a) => Some(a),
                    Err(why) => return Ok((None, respond(operation, 409, json!(why)))),
                },
                None => None,
            };
            let plan = match resumes {
                Some(n) => match resumed(env, s, n).await? {
                    Ok(before) => format!("{before}\n\n{plan}"),
                    Err(why) => return Ok((None, respond(operation, 409, json!(why)))),
                },
                None => plan,
            };
            let mut change = Change::propose(0, &slug, &title, &plan, scope, &who.subject);
            change.parent = resumes;
            change.mandate = mandate;
            change.session = session;
            change.affects = affects;
            change.full_asked = full.then(|| who.subject.clone());
            change.after.clone_from(&built);
            let proposed = propose(env, s, operation, change).await?;
            if let Some(n) = resumes {
                tasks::close_for(env, s, n).await?;
            }
            if let (Some(a), Some(number)) =
                (built, proposed.0.as_deref().and_then(|n| n.parse().ok()))
            {
                let reason = format!("built on !{}", a.number);
                set_yield(env, s, number, a.number, &who.subject, &reason).await?;
            }
            Ok(proposed)
        }
        Routed::Event {
            number,
            event: Event::Merge { by },
        } => {
            let (status, v) = merges::ask(env, s, number, &by).await?;
            Ok((Some(number.to_string()), respond(operation, status, v)))
        }
        Routed::Event {
            number,
            event: event @ Event::Approve { .. },
        } => approve(env, s, operation, number, &event, hold).await,
        Routed::Event { number, event } => {
            let body = serde_json::to_value(&event)?;
            let (status, v) =
                to_change(env, &s.repo, number, Method::Post, "/event", Some(&body)).await?;
            Ok((
                Some(number.to_string()),
                respond(operation, status, json!({ "change": v })),
            ))
        }
        Routed::BreakGlass { number } => break_glass(env, s, operation, number).await,
        Routed::CiWindow { hours } => Ok(answered(operation, ci_window::run(env, s, hours).await?)),
        Routed::Status { number } => {
            let (status, mut v) =
                to_change(env, &s.repo, number, Method::Get, "/status", None).await?;
            if let Ok(c) = serde_json::from_value::<Change>(v["change"].clone()) {
                let q = stages::queue(env, s).await;
                v["stage"] = serde_json::to_value(stages::of(env, s, &c, &q).await)?;
            }
            Ok((Some(number.to_string()), respond(operation, status, v)))
        }
        Routed::DeployRetry => {
            let req = json_request("https://main/retry", Method::Post, None, None)?;
            let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
            let v: Value = r.json().await?;
            Ok((None, respond(operation, r.status_code(), v)))
        }
        Routed::DeployStatus => deploy_status(env, s, operation).await,
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
            let rows = stages::list(env, s, rows).await?;
            Ok((None, respond(operation, 200, json!({ "changes": rows }))))
        }
        Routed::Note { number, text } => {
            let said = format!("a note on !{number}: {text}");
            events::append(env, s, "change.note", Some(number), None, &said, false).await?;
            Ok((
                Some(number.to_string()),
                respond(operation, 200, json!({ "noted": number })),
            ))
        }
        Routed::Handoff {
            number,
            session,
            note,
        } => {
            let said = format!("!{number} is offered to {session}: {note}");
            tasks::add(
                env,
                s,
                "handoff",
                number,
                None,
                Some(session.clone()),
                &said,
            )
            .await?;
            events::append(
                env,
                s,
                "change.handed",
                Some(number),
                Some(session),
                &said,
                false,
            )
            .await?;
            Ok((
                Some(number.to_string()),
                respond(operation, 200, json!({ "offered": number })),
            ))
        }
        Routed::Yield {
            number,
            to,
            reason,
            off,
        } => change_yield(env, s, who, operation, number, to, &reason, off).await,
        Routed::Pin { number, off } => {
            let body = json!({ "number": number, "off": off });
            let (status, v) = mandates::to_instance(env, s, "/pins/set", &body).await?;
            Ok((
                Some(number.to_string()),
                respond(operation, status, json!({ "pinned": v })),
            ))
        }
        Routed::Evictable => {
            let req = json_request("https://main/evictions/last", Method::Get, None, None)?;
            let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
            let judged: Value = r.json().await.unwrap_or(Value::Null);
            let result = json!({
                "candidates": evictable(env, s).await?,
                "judged": judged,
                "evicts": "nothing yet: verdicts only",
            });
            Ok((None, respond(operation, 200, result)))
        }
        Routed::SessionStatus { args } => {
            let (status, v) = stages::session(env, s, &args).await?;
            Ok((None, respond(operation, status, v)))
        }
        Routed::SessionEnd { args } => Ok(answered(operation, tickets::end(env, s, &args).await?)),
        Routed::SessionStart { args } => {
            Ok(answered(operation, tickets::start(env, s, &args).await?))
        }
        Routed::Digest => Ok((
            None,
            respond(operation, 200, push::show(env, s, who.human).await?),
        )),
        Routed::AppleCheck => Ok((None, respond(operation, 200, apple::check(env).await?))),
        Routed::IosRelease | Routed::IosStatus => {
            let (method, path) = if matches!(routed, Routed::IosRelease) {
                (Method::Post, "https://main/ios/release")
            } else {
                (Method::Get, "https://main/ios/status")
            };
            let req = json_request(path, method, None, None)?;
            let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
            let status = r.status_code();
            let v: Value = if (200..300).contains(&status) {
                r.json().await?
            } else {
                json!({ "error": r.text().await? })
            };
            Ok((None, respond(operation, status, v)))
        }
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

/// `deploy.status`: `main`'s pipeline, its last log, and the owner's CI window when open.
async fn deploy_status(
    env: &Env,
    s: &Settings,
    operation: &str,
) -> Result<(Option<String>, WireResponse)> {
    let req = json_request("https://main/status", Method::Get, None, None)?;
    let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
    let mut v: Value = r.json().await?;
    v["pipeline"]["last_log"] = json!(last_log(env, &v).await?);
    v["ci_window"] = json!(ci_window::current(env, s).await?);
    Ok((None, respond(operation, r.status_code(), v)))
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

/// Parks change `from` behind `to`, as `by` for `reason`: on the change, in the Instance's yield
/// map, and in the event log.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn set_yield(
    env: &Env,
    s: &Settings,
    from: u32,
    to: u32,
    by: &str,
    reason: &str,
) -> Result<(u16, Value)> {
    let body = json!({ "to": to, "by": by, "reason": reason });
    let (status, v) = to_change(env, &s.repo, from, Method::Post, "/yield", Some(&body)).await?;
    if status == 200 {
        let session = v["session"].as_str().map(str::to_string);
        recorded_yield(env, s, from, to, reason, session).await?;
    }
    Ok((status, v))
}

/// A yield of `from` to `to`, already on the change, kept in the Instance and logged.
async fn recorded_yield(
    env: &Env,
    s: &Settings,
    from: u32,
    to: u32,
    reason: &str,
    session: Option<String>,
) -> Result<()> {
    mandates::to_instance(env, s, "/yields/set", &json!({ "from": from, "to": to })).await?;
    let said = format!("!{from} yields to !{to}: {reason}");
    events::append(env, s, "change.yielded", Some(from), session, &said, false).await
}

/// The open changes whose scope meets `c`'s and that have pushed, as the bound job's
/// [`control::OVERLAPS`]; see the 2026-11-12 ADR §5's conflict forecasts.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn overlapping(env: &Env, c: &Change) -> Result<Value> {
    let s = Settings::from(env)?;
    let rows = platform::d1_rows(env, &control::open(&s.instance, &s.repo)).await?;
    let mine: Vec<&String> = c.scope.iter().chain(&c.affects).collect();
    let others: Vec<Value> = control::open_rows(&rows)
        .into_iter()
        .filter(|o| o.number != c.number && !matches!(o.state.as_str(), "proposed" | "planned"))
        .filter(|o| {
            o.scope
                .iter()
                .chain(&o.affects)
                .any(|g| mine.iter().any(|m| crate::sessions::meet(m, g)))
        })
        .map(|o| json!({ "number": o.number, "branch": format!("change/{}-{}", o.number, o.slug), "state": o.state }))
        .collect();
    Ok(Value::Array(others))
}

/// `c` after its bound job's `forecast`: for the first conflict, `c` yields when it is less far
/// along, or the other change is asked to; nothing when `c` already waits on one.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn apply_forecast(env: &Env, c: Change, forecast: &[Value]) -> Result<Change> {
    use crate::core::change::forecast::yields;
    let s = Settings::from(env)?;
    if c.yielded.is_some() {
        return Ok(c);
    }
    for f in forecast {
        let Some(n) = f["number"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        let (status, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
        let state = v["change"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if status != 200 || matches!(state.as_str(), "merged" | "closed") {
            continue;
        }
        let paths: Vec<&str> = f["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let reason = format!("forecast: they conflict in {}", paths.join(", "));
        if yields((c.number, c.state.name()), (n, &state)) {
            let Ok(parked) = c.clone().yield_to(n, "forecast", &reason, now()) else {
                continue;
            };
            recorded_yield(env, &s, c.number, n, &reason, c.session.clone()).await?;
            return Ok(parked);
        }
        if v["change"]["yielded"].is_null() {
            set_yield(env, &s, n, c.number, "forecast", &reason).await?;
        }
        return Ok(c);
    }
    Ok(c)
}

/// `change.yield`: checks the change waited for is open, then parks `number` behind it, or
/// ends its yield with `off`.
#[allow(clippy::too_many_arguments)]
async fn change_yield(
    env: &Env,
    s: &Settings,
    who: &Identity,
    operation: &str,
    number: u32,
    to: Option<u32>,
    reason: &str,
    off: bool,
) -> Result<(Option<String>, WireResponse)> {
    let key = Some(number.to_string());
    if off {
        let (status, v) = to_change(
            env,
            &s.repo,
            number,
            Method::Post,
            "/yield",
            Some(&json!({ "off": true })),
        )
        .await?;
        mandates::to_instance(env, s, "/yields/clear", &json!({ "from": number })).await?;
        return Ok((key, respond(operation, status, v)));
    }
    let Some(to) = to else {
        return Ok((
            key,
            respond(operation, 400, json!("`to` names the change to yield to")),
        ));
    };
    let (status, v) = to_change(env, &s.repo, to, Method::Get, "/status", None).await?;
    let open = status == 200 && !matches!(v["change"]["state"].as_str(), Some("merged" | "closed"));
    if !open {
        return Ok((
            key,
            respond(
                operation,
                409,
                json!(format!("!{to} is not an open change")),
            ),
        ));
    }
    let (status, v) = set_yield(env, s, number, to, &who.subject, reason).await?;
    Ok((key, respond(operation, status, v)))
}

/// Each open change as an eviction candidate, from D1, the tickets, the merge queue and the
/// waiting approvals; see `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §4.
async fn evictable(env: &Env, s: &Settings) -> Result<Vec<Value>> {
    use crate::core::change::eviction::{Facts, Holds, Session, candidate};
    let rows = platform::d1_rows(env, &control::evictable(&s.instance, &s.repo)).await?;
    let (_, t) = mandates::to_instance(env, s, "/tickets/list", &json!({})).await?;
    let tickets: crate::tickets::Tickets = serde_json::from_value(t).unwrap_or_default();
    let (_, q) = mandates::to_instance(env, s, "/merges/list", &json!({})).await?;
    let queue: crate::merges::MergeQueue = serde_json::from_value(q).unwrap_or_default();
    let mut queued: Vec<u32> = queue.waiting.iter().map(|e| e.number).collect();
    queued.extend(queue.merging.as_ref().map(|e| e.number));
    let (_, pins) = mandates::to_instance(env, s, "/pins/list", &json!({})).await?;
    let pinned: Vec<u32> = serde_json::from_value(pins).unwrap_or_default();
    let waiting: Vec<crate::approve::Waiting> =
        serde_json::from_value(elicit::inbox(env, s, "/approvals/list", &json!({})).await?)
            .unwrap_or_default();
    let t = now();
    Ok(rows
        .iter()
        .filter_map(|r| {
            let number = u32::try_from(r["number"].as_u64()?).ok()?;
            let session = r["session"].as_str().map(|label| Session {
                label: label.to_string(),
                live: tickets.live(label, t).is_some(),
                gone_for: tickets.gone_for(label, t),
            });
            let f = Facts {
                number,
                slug: r["slug"].as_str().unwrap_or_default().to_string(),
                state: r["state"].as_str().unwrap_or_default().to_string(),
                head: r["head"].as_str().map(str::to_string),
                updated: r["updated"].as_u64().unwrap_or_default(),
                session,
                holds: Holds {
                    pinned: pinned.contains(&number),
                    approval_open: waiting.iter().any(|w| w.number == number),
                    queued: queued.contains(&number),
                },
            };
            Some(json!({ "number": number, "fields": candidate(&f, t) }))
        })
        .collect())
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
    if is(crate::cobot::ship_wire::ACCESS_ID_VAR) {
        return Some(Response::error(
            "the deployer's token reads /healthz and /healthz/smoke only",
            403,
        ));
    }
    if is(crate::cobot::ship_wire::BUILDS_ACCESS_ID_VAR) {
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
    let mut probes = json!({
        "d1": d1, "instance": instance, "main": main, "tools": tools,
        "change_status": change, "deploy_status": deploy,
    });
    // The docs site, once its bucket is bound.
    if let Ok(bucket) = env.bucket(docs::BINDING) {
        let found = bucket.head("index.html").await.is_ok_and(|o| o.is_some());
        probes["docs"] = json!(found);
    }
    let ok = !version.is_empty()
        && probes
            .as_object()
            .is_some_and(|m| m.values().all(|v| v == true));
    Ok(
        Response::from_json(&json!({ "ok": ok, "version": version, "probes": probes }))?
            .with_status(if ok { 200 } else { 503 }),
    )
}

/// `GET /builds/<sha>/<file>`: `main`'s `pigeon` or `pigeon-sentinel` built at that commit, or
/// its test map, from R2.
async fn build(env: &Env, path: &str) -> Result<Response> {
    let Some((sha, binary)) = crate::cobot::ship_wire::build_path(path) else {
        return Response::error("not found", 404);
    };
    let key = if binary == crate::cobot::ship_wire::SENTINEL_BINARY {
        crate::cobot::ship_wire::sentinel_key(sha)
    } else if binary == crate::cobot::ship_wire::TEST_MAP_FILE {
        crate::testmaps::key(sha)
    } else {
        crate::cobot::ship_wire::build_key(sha)
    };
    let Some(object) = env.bucket("CI_LOGS")?.get(key).execute().await? else {
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
pub(super) async fn latest_build(env: &Env) -> Option<crate::cobot::worker::JobsBuild> {
    let host = env.var(crate::cobot::ship_wire::HOST_VAR).ok()?.to_string();
    let access = (
        platform::secret(env, crate::cobot::ship_wire::BUILDS_ACCESS_ID_VAR).ok()?,
        platform::secret(env, crate::cobot::ship_wire::BUILDS_ACCESS_SECRET_VAR).ok()?,
    );
    let repo = env.var("REPO").ok()?.to_string();
    let mut r = main_stub(env, &repo)
        .ok()?
        .fetch_with_str("https://main/status")
        .await
        .ok()?;
    let v: Value = r.json().await.ok()?;
    let b: control::Build = serde_json::from_value(v["pipeline"]["latest_build"].clone()).ok()?;
    let sentinel = b.sentinel_sha256.map(|sum| {
        (
            format!(
                "https://{host}/builds/{}/{}",
                b.sha,
                crate::cobot::ship_wire::SENTINEL_BINARY
            ),
            sum,
        )
    });
    let test_map =
        serde_json::from_value::<crate::testmaps::Map>(v["test_map"]["latest_map"].clone())
            .ok()
            .map(|m| {
                let file = crate::cobot::ship_wire::TEST_MAP_FILE;
                (format!("https://{host}/builds/{}/{file}", m.sha), m.sha256)
            });
    Some(crate::cobot::worker::JobsBuild {
        url: format!("https://{host}/builds/{}/pigeon", b.sha),
        sha256: b.sha256,
        access,
        sentinel,
        test_map,
    })
}

pub(super) fn main_stub(env: &Env, repo: &str) -> Result<worker::Stub> {
    env.durable_object("MAIN_RUN")?
        .id_from_name(&format!("{repo}#main"))?
        .get_stub()
}

/// The hourly digest; see
/// `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §4.
#[event(scheduled)]
async fn scheduled(_event: worker::ScheduledEvent, env: Env, _ctx: worker::ScheduleContext) {
    // The mirror's hourly reconcile; see
    // .claude/decisions/2026-11-08-development-github-mirror-on-merge.md §1.
    if github::mirror(&env).is_some()
        && let Err(e) = reconcile_mirror(&env).await
    {
        worker::console_error!("mirror reconcile: {e}");
    }
    if let Err(e) = push::hourly(&env).await {
        worker::console_error!("digest: {e}");
    }
    if let Err(e) = insights::collect(&env).await {
        worker::console_error!("insights: {e}");
    }
    // Every open change judged by main's eviction policies; see
    // .claude/decisions/2026-11-10-development-change-slot-eviction-policy.md §5.
    // Act on the last run's verdicts first, then judge again for the next hour.
    if let Ok(s) = Settings::from(&env)
        && let Err(e) = act_on_evictions(&env, &s).await
    {
        worker::console_error!("evictions: {e}");
    }
    if let Err(e) = judge_evictions(&env).await {
        worker::console_error!("evictions: {e}");
    }
}

/// The `Instance`'s tickets, yields and pins.
async fn sessions_store(state: &State, path: &str, body: &Value) -> Result<Response> {
    if path.starts_with("/tickets/") {
        tickets::serve(state, path, body).await
    } else if path.starts_with("/pins/") {
        pins(state, path, body).await
    } else {
        yields(state, path, body).await
    }
}

/// The `Instance`'s yields, each change to the one it waits for: `/yields/set` (`from`, `to`),
/// `/yields/clear` (`from`), and `/yields/take` (`to`), which answers and forgets the changes
/// waiting for `to`.
async fn yields(state: &State, path: &str, body: &Value) -> Result<Response> {
    const YIELDS: &str = "yields";
    let storage = state.storage();
    let mut all: std::collections::BTreeMap<u32, u32> =
        storage.get(YIELDS).await?.unwrap_or_default();
    let n = |k: &str| body[k].as_u64().and_then(|n| u32::try_from(n).ok());
    let answer: Vec<u32> = match (path, n("from"), n("to")) {
        ("/yields/set", Some(from), Some(to)) => {
            all.insert(from, to);
            Vec::new()
        }
        ("/yields/clear", Some(from), _) => {
            all.remove(&from);
            Vec::new()
        }
        ("/yields/take", _, Some(to)) => {
            let waiting: Vec<u32> = all
                .iter()
                .filter(|(_, t)| **t == to)
                .map(|(f, _)| *f)
                .collect();
            all.retain(|_, t| *t != to);
            waiting
        }
        _ => return Response::error("a yield names its changes", 400),
    };
    storage.put(YIELDS, &all).await?;
    Response::from_json(&answer)
}

/// Ends every yield to `c`, which merged or closed: each waiting change is unyielded and its
/// session told, to restack at its next merge turn.
///
/// # Errors
///
/// When a binding fails.
pub(super) async fn release_yields(env: &Env, c: &Change) -> Result<()> {
    let s = Settings::from(env)?;
    let (_, v) = mandates::to_instance(env, &s, "/yields/take", &json!({ "to": c.number })).await?;
    let waiting: Vec<u32> = serde_json::from_value(v).unwrap_or_default();
    let off = json!({ "off": true, "ended": c.number, "head": c.head });
    for n in waiting {
        let (status, v) = to_change(env, &s.repo, n, Method::Post, "/yield", Some(&off)).await?;
        if status != 200 {
            continue;
        }
        let session = v["session"].as_str().map(str::to_string);
        let note = format!(
            "!{} {}, so !{n}'s yield to it ended; merge !{n} when ready",
            c.number,
            c.state.name()
        );
        events::append(
            env,
            &s,
            "change.unyielded",
            Some(n),
            session.clone(),
            &note,
            false,
        )
        .await?;
        tasks::add(env, &s, "yield-ended", n, Some(c.number), session, &note).await?;
    }
    Ok(())
}

/// The `Instance`'s pins: the changes the owner holds from eviction; `/pins/list` and
/// `/pins/set` (`number`, `off`).
async fn pins(state: &State, path: &str, body: &Value) -> Result<Response> {
    const PINS: &str = "pins";
    let storage = state.storage();
    let mut pinned: std::collections::BTreeSet<u32> = storage.get(PINS).await?.unwrap_or_default();
    if path == "/pins/set" {
        let Some(n) = body["number"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            return Response::error("a pin names a change", 400);
        };
        if body["off"].as_bool() == Some(true) {
            pinned.remove(&n);
        } else {
            pinned.insert(n);
        }
        storage.put(PINS, &pinned).await?;
    }
    Response::from_json(&pinned)
}

/// Acts on the last verdicts: each change a policy granted, still as it was judged, is evicted;
/// see `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §5-§6.
async fn act_on_evictions(env: &Env, s: &Settings) -> Result<()> {
    use crate::core::change::eviction::{incomplete, pick};
    let req = json_request("https://main/evictions/last", Method::Get, None, None)?;
    let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
    let e: control::Evictions = r.json().await.unwrap_or_default();
    let Some(judged) = e.last.filter(|j| j.passed) else {
        return Ok(());
    };
    let still: Vec<Value> = evictable(env, s).await?;
    for (n, policy) in pick(&judged.verdicts, judged.at, now()) {
        // Judged as it still is: open, unpinned, unqueued, no page waiting.
        let fresh = still
            .iter()
            .find(|c| c["number"].as_u64() == Some(u64::from(n)));
        let holds = |k: &str| fresh.is_some_and(|c| c["fields"][k] == "true");
        if fresh.is_none() || holds("pinned") || holds("queued") || holds("approval_open") {
            continue;
        }
        let (status, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
        let Some(change) = (status == 200)
            .then(|| serde_json::from_value::<Change>(v["change"].clone()).ok())
            .flatten()
        else {
            continue;
        };
        let body = json!({ "policy": format!("policy:{policy}") });
        let (status, _) = to_change(env, &s.repo, n, Method::Post, "/evict", Some(&body)).await?;
        if status != 200 {
            continue;
        }
        let said = format!("!{n} `{}` was evicted under {policy}", change.title);
        events::append(
            env,
            s,
            "change.evicted",
            Some(n),
            change.session.clone(),
            &said,
            false,
        )
        .await?;
        if incomplete(&change) {
            let left = crate::core::change::eviction::remains(&change)
                .left
                .join("; ");
            let note = format!("{said}. Left: {left}. Should a new session finish it?");
            tasks::add(env, s, "resume", n, None, None, &note).await?;
            push::notify(env, s, "Evicted, unfinished", &note, "/approvals").await;
        }
    }
    Ok(())
}

/// The opening of a plan that resumes evicted change `n`: what it did and has left; refused
/// unless `n` was evicted.
/// The open change `n` a new change is built on; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.
async fn built_on(
    env: &Env,
    s: &Settings,
    n: u32,
) -> Result<std::result::Result<crate::changes::After, String>> {
    let (status, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    let open = (status == 200)
        .then(|| serde_json::from_value::<Change>(v["change"].clone()).ok())
        .flatten()
        .filter(|c| {
            !matches!(
                c.state,
                crate::changes::State::Merged | crate::changes::State::Closed
            )
        });
    Ok(open
        .map(|c| crate::changes::After {
            number: n,
            branch: c.branch(),
            head: None,
        })
        .ok_or_else(|| format!("!{n} is not an open change to build on")))
}

async fn resumed(env: &Env, s: &Settings, n: u32) -> Result<std::result::Result<String, String>> {
    let (status, v) = to_change(env, &s.repo, n, Method::Get, "/status", None).await?;
    let change = (status == 200)
        .then(|| serde_json::from_value::<Change>(v["change"].clone()).ok())
        .flatten();
    let Some(e) = change.as_ref().and_then(|c| c.evicted.clone()) else {
        return Ok(Err(format!(
            "!{n} was not evicted, so there is nothing to resume"
        )));
    };
    let branch = change.map(|c| c.branch()).unwrap_or_default();
    Ok(Ok(format!(
        "Resumes !{n}, evicted under {}; its head {} is kept on {branch}.\n\nDone: {}\n\nLeft: {}",
        e.policy,
        e.head.as_deref().unwrap_or("(none)"),
        e.remains.done.join("; "),
        e.remains.left.join("; ")
    )))
}

/// Sends every open change's candidate to `MainRun`'s eviction job.
async fn judge_evictions(env: &Env) -> Result<()> {
    let s = Settings::from(env)?;
    let candidates = evictable(env, &s).await?;
    if candidates.is_empty() {
        return Ok(());
    }
    let body = json!({ "candidates": candidates });
    let req = json_request(
        "https://main/evictions/run",
        Method::Post,
        Some(&body),
        None,
    )?;
    let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
    if !(200..300).contains(&r.status_code()) {
        return Err(r.text().await.unwrap_or_default().into());
    }
    Ok(())
}

async fn reconcile_mirror(env: &Env) -> Result<()> {
    let s = Settings::from(env)?;
    let req = json_request(
        "https://main/reconcile",
        Method::Post,
        Some(&json!({})),
        None,
    )?;
    let mut r = platform::call(env, &main_stub(env, &s.repo)?, req).await?;
    if !(200..300).contains(&r.status_code()) {
        return Err(r.text().await.unwrap_or_default().into());
    }
    Ok(())
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
        if path == "/apns/token" {
            let body: Value = req.json().await?;
            return apns::serve(&self.state, body).await;
        }
        if path.starts_with("/windows/") {
            let body: Value = req.json().await?;
            return mandates::serve_windows(&self.state, &path, body).await;
        }
        if path.starts_with("/mandates/") {
            let body: Value = req.json().await?;
            return mandates::serve(&self.state, &path, body).await;
        }
        if path.starts_with("/devices/") {
            let body: Value = req.json().await?;
            return devices::serve(&self.state, &path, body).await;
        }
        if path.starts_with("/passkeys/") {
            let body: Value = req.json().await?;
            return passkeys::serve(&self.state, &path, body).await;
        }
        if ["/tickets/", "/yields/", "/pins/"]
            .iter()
            .any(|p| path.starts_with(p))
        {
            let body: Value = req.json().await.unwrap_or(Value::Null);
            return sessions_store(&self.state, &path, &body).await;
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
        if path.starts_with("/ci-window/") {
            let body: Value = req.json().await?;
            return ci_window::serve(&self.state, &path, &body).await;
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
        let queue: Queue = storage.get(QUEUE).await?.unwrap_or_default();
        let (queue, answer) = match (method, segments.as_slice(), submitted) {
            (Method::Get, ["runs"], _) => return Response::from_json(&queue),
            (Method::Post, ["runs"], Some(run)) => match queue.enqueue(run) {
                Ok(next) => {
                    let answer = Response::from_json(&next);
                    (next, answer)
                }
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", "start"], _) => {
                let (next, running) = queue.start();
                (next, Response::from_json(&running))
            }
            (Method::Post, ["runs", id, "finish"], _) => match queue.finish(id) {
                Ok((next, run)) => (next, Response::from_json(&run)),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            (Method::Post, ["runs", id, "lost"], _) => match queue.lost(id) {
                Ok((next, Lost::Retry(run))) => (
                    next,
                    Response::from_json(&serde_json::json!({ "retry": run })),
                ),
                Ok((next, Lost::Failed(run))) => (
                    next,
                    Response::from_json(&serde_json::json!({ "failed": run })),
                ),
                Err(e) => return Response::error(e.to_string(), 409),
            },
            _ => return Response::error("not found", 404),
        };
        storage.put(QUEUE, &queue).await?;
        answer
    }
}
