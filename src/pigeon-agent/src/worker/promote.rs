//! A promote, as `MainRun` drives it: drain, adopt and parity on the standby, the fenced flip, the watch from the old colour, and flip-back; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20 amendments.

use serde_json::Value;
use worker::{Env, Method, Request, RequestInit, Response, Result, State};

use super::platform::{self, Settings, now};
use crate::core::deploy::standby::{Checked, VOLATILE};
use crate::core::lease::Lease;
use crate::core::lease::step::{self, Input, Phase, Promote, Step};

/// The promote in progress; never journaled, so a colour's adopt never carries another's.
pub const PROMOTE: &str = "promote";
/// A deploy's passed standby, waiting for the next tick.
pub const READY: &str = "promote.ready";
/// The owner's `lease.flip`, naming the colour, waiting for the next tick.
pub const FLIP: &str = "promote.flip";
/// The owner's `promote.hold`: a passed standby doesn't promote while it is set; held until the owner first releases it, so the first cutover is an owner's `lease.flip`.
pub const HOLD: &str = "promote.hold";

/// How often a promote ticks.
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// How far back a running job's D1 row counts for a drain.
const RUNNING_SINCE_S: u64 = 30 * 60;

/// The binding a colour is reached through, as the front names it.
fn binding(color: &str) -> String {
    color.to_ascii_uppercase().replace('-', "_")
}

/// Calls `color`'s `path` through its service binding with the colour token: its status and body.
async fn call(
    env: &Env,
    color: &str,
    method: Method,
    path: &str,
) -> std::result::Result<(u16, String), String> {
    let service = env
        .service(&binding(color))
        .map_err(|_| format!("no binding for {color}"))?;
    let token = super::checker::signing_key(env).map_err(|e| e.to_string())?;
    let headers = worker::Headers::new();
    headers
        .set(crate::core::lease::COLOUR_TOKEN, &token)
        .map_err(|e| e.to_string())?;
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    let req = Request::new_with_init(&format!("https://colour{path}"), &init)
        .map_err(|e| e.to_string())?;
    let mut r = service
        .fetch_request(req)
        .await
        .map_err(|e| e.to_string())?;
    Ok((r.status_code(), r.text().await.unwrap_or_default()))
}

/// Each object's `held` digests in a `/healthz/parity` answer, without [`VOLATILE`] keys.
fn held(parity: &Value) -> std::collections::BTreeMap<String, Value> {
    let mut out = std::collections::BTreeMap::new();
    if let Some(objects) = parity["objects"].as_object() {
        for (o, v) in objects {
            let mut h = v["held"].clone();
            if let Some(m) = h.as_object_mut() {
                for k in VOLATILE {
                    m.remove(*k);
                }
            }
            out.insert(o.clone(), h);
        }
    }
    out
}

/// The standby `to` adopts the journal, then holds what this primary holds, object by object.
async fn ready(
    state: &State,
    env: &Env,
    s: &Settings,
    to: &str,
) -> std::result::Result<(), String> {
    let (status, body) = call(env, to, Method::Post, "/healthz/adopt").await?;
    if status != 200 {
        return Err(format!("adopt: status {status}: {body}"));
    }
    let (status, body) = call(env, to, Method::Get, "/healthz/parity").await?;
    if status != 200 && status != 503 {
        return Err(format!("parity: status {status}"));
    }
    let theirs: Value = serde_json::from_str(&body).map_err(|e| format!("parity: {e}"))?;
    let ours_passkeys = super::passkey_ids(env, s).await;
    if ours_passkeys.is_null() || theirs["passkeys"] != ours_passkeys {
        return Err(format!(
            "{to} holds other passkeys than this colour, or none could be read"
        ));
    }
    let ours_vars = super::matched_vars(env);
    let vars: Vec<&String> = ours_vars
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, v)| theirs["vars"].get(k.as_str()) != Some(*v))
        .map(|(k, _)| k)
        .collect();
    if !vars.is_empty() {
        let names: Vec<&str> = vars.iter().map(|k| k.as_str()).collect();
        return Err(format!("{to} holds other vars: {}", names.join(", ")));
    }
    let ours_keys = super::key_fingerprints(env);
    let keys: Vec<&String> = ours_keys
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, v)| theirs["keys"].get(k.as_str()) != Some(*v))
        .map(|(k, _)| k)
        .collect();
    if !keys.is_empty() {
        let names: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
        return Err(format!("{to} holds other secrets: {}", names.join(", ")));
    }
    let ours = held_here(state, env, s).await?;
    let (t, o) = (held(&theirs), held(&ours));
    let differs: Vec<&String> = o.keys().filter(|k| t.get(*k) != o.get(*k)).collect();
    if differs.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "parity: {to} holds other state for {}",
            differs
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// The jobs a drain waits for: this colour's deploy, and the code checks D1 shows running.
async fn running(env: &Env, s: &Settings, deploying: bool) -> Result<u64> {
    let since = now().saturating_sub(RUNNING_SINCE_S);
    let rows = platform::d1_rows(
        env,
        &crate::core::lease::running_jobs(&s.instance, &s.repo, since),
    )
    .await?;
    let n = rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(n + u64::from(deploying))
}

/// One minute of the watch: `to` answers `/healthz` through its binding, and the lease, which the front reads, still names it.
async fn watched(env: &Env, to: &str) -> bool {
    let up = matches!(call(env, to, Method::Get, "/healthz").await, Ok((200, _)));
    let named = super::lease::current(env)
        .await
        .is_some_and(|l| l.color == to);
    up && named
}

/// Records a deploy's standby check: a passed one waits for the next tick to promote.
///
/// # Errors
///
/// When storage fails.
pub async fn standby_checked(state: &State, checked: &Checked) -> Result<()> {
    if checked.passed {
        state.storage().put(READY, checked).await?;
    }
    Ok(())
}

/// Logs `kind` to the event log, and pushes the owner for an abort or a flip-back.
async fn event(env: &Env, s: &Settings, kind: &str, summary: &str) {
    if let Err(e) = super::events::append(env, s, kind, None, None, summary, false).await {
        worker::console_error!("lease event: {e}");
    }
    if kind == "lease.flipped_back" || kind == "lease.promote_aborted" {
        super::push::notify(env, s, "Promote stopped", summary, "/approvals").await;
    }
}

/// One tick of the promote: starts one from a passed standby or the owner's flip, drains, flips, watches or flips back. Whether a promote is in progress after it, which holds new deploys.
///
/// # Errors
///
/// When storage or D1 fails.
pub async fn tick(state: &State, env: &Env, deploying: bool) -> Result<bool> {
    let storage = state.storage();
    let s = Settings::from(env)?;
    let Some(lease) = super::lease::current(env).await else {
        return Ok(false);
    };
    let mine = super::lease::color(env);
    let promote: Option<Promote> = storage.get(PROMOTE).await?;
    let Some(input) = input(state, env, &s, &lease, promote.as_ref(), deploying).await? else {
        return Ok(false);
    };
    let mut st: Step = step::step(&lease, promote.as_ref(), &input, now());
    let mut note = String::new();
    if st.event == Some("lease.promoted")
        && let Some(p) = &promote
        && let Err(why) = ready(state, env, &s, &p.to).await
    {
        st = Step {
            promote: None,
            lease: Some(Lease {
                state: "serving".into(),
                ..lease.clone()
            }),
            event: Some("lease.promote_aborted"),
        };
        note = why;
    }
    if st.event == Some("lease.flipped_back") {
        // The old colour takes the journal back before it takes the lease back.
        match super::adopt(env, &s, true).await {
            Ok(r) if r.status_code() == 200 => {}
            Ok(r) => note = format!("its own adopt answered {}", r.status_code()),
            Err(e) => note = format!("its own adopt failed: {e}"),
        }
        // `MainRun` adopts inline, as this tick runs in it, and only while this colour doesn't act,
        // as `adopt_into` checks.
        let main = crate::core::journal::main_object(&s.instance);
        match super::restored(env, &main).await {
            Ok(_) if super::lease::acting(env).await => {
                note = format!("{note} its MainRun didn't adopt: this colour acts");
            }
            Ok(v) => {
                super::put_adopted(&storage, crate::core::journal::MAIN_KEYS, &v).await?;
            }
            Err(e) => note = format!("{note} its MainRun didn't adopt: {e}"),
        }
    }
    if let Some(new) = &st.lease {
        platform::d1(env, &[crate::core::lease::write(lease.epoch, new)]).await?;
        if super::lease::current(env).await.as_ref() != Some(new) {
            // Another writer moved the lease first; this promote is over.
            storage.delete(PROMOTE).await?;
            let why = "the lease moved under the promote";
            event(env, &s, "lease.promote_aborted", why).await;
            return Ok(false);
        }
    }
    match &st.promote {
        Some(p) => storage.put(PROMOTE, p).await?,
        None => {
            storage.delete(PROMOTE).await?;
        }
    }
    announce(env, &s, &st, promote.as_ref(), &mine, note).await;
    Ok(st.promote.is_some())
}

/// What this tick saw, for `step`; None when there is nothing to step.
async fn input(
    state: &State,
    env: &Env,
    s: &Settings,
    lease: &Lease,
    promote: Option<&Promote>,
    deploying: bool,
) -> Result<Option<Input>> {
    let storage = state.storage();
    let mine = super::lease::color(env);
    Ok(Some(match promote {
        None if lease.color != mine => return Ok(None),
        None if lease.state != "serving" => {
            // A promote that lost its record: the lease serves again.
            let back = Lease {
                state: "serving".into(),
                ..lease.clone()
            };
            platform::d1(env, &[crate::core::lease::write(lease.epoch, &back)]).await?;
            let why = format!("{mine} serves again: its promote had no record");
            event(env, s, "lease.promote_aborted", &why).await;
            return Ok(None);
        }
        None => {
            if let Some(c) = storage.get::<String>(FLIP).await? {
                storage.delete(FLIP).await?;
                Input::Flip { color: c }
            } else if let Some(c) = storage.get::<Checked>(READY).await? {
                storage.delete(READY).await?;
                let hold = storage.get::<bool>(HOLD).await?.unwrap_or(true);
                Input::StandbyPassed {
                    color: c.color,
                    hold,
                }
            } else {
                return Ok(None);
            }
        }
        Some(p) if p.from != mine => {
            storage.delete(PROMOTE).await?;
            return Ok(None);
        }
        Some(p) => match p.phase {
            Phase::Draining => Input::Running(running(env, s, deploying).await?),
            Phase::Watching => Input::Probed(watched(env, &p.to).await),
        },
    }))
}

/// Logs the transition `st` made, resuming the colour that now serves.
async fn announce(
    env: &Env,
    s: &Settings,
    st: &Step,
    promote: Option<&Promote>,
    mine: &str,
    mut note: String,
) {
    let (from, to) = promote.map_or((mine.to_string(), String::new()), |p| {
        (p.from.clone(), p.to.clone())
    });
    let summary = match st.event {
        Some("lease.promoting") => {
            let to = st.promote.as_ref().map_or("", |p| p.to.as_str());
            format!("{from} drains to promote {to}")
        }
        Some("lease.promoted") => {
            if let Err(e) = call(env, &to, Method::Post, "/healthz/resume").await {
                note = format!("resume: {e}");
            }
            format!("{to} serves, from {from}; watched for 10 minutes {note}")
        }
        Some("lease.flipped_back") => {
            let _ = super::resume(env, s, true).await;
            format!("{to} failed its watch; {from} serves again {note}")
        }
        Some(_) => format!("{from} serves on; {to} not promoted: {note}"),
        None if promote.is_some() && st.promote.is_none() => format!("{to} passed its watch"),
        None => return,
    };
    let kind = st.event.unwrap_or("lease.watched");
    event(env, s, kind, &summary).await;
}

/// Whether this colour runs a promote it started, as the old colour does during the watch.
///
/// # Errors
///
/// When storage fails.
pub async fn watching(state: &State) -> Result<bool> {
    Ok(state.storage().get::<Promote>(PROMOTE).await?.is_some())
}

/// The answer a standby's alarm gives while it watches.
///
/// # Errors
///
/// When storage fails.
pub async fn watch_tick(state: &State, env: &Env) -> Result<Response> {
    if let Err(e) = tick(state, env, false).await {
        worker::console_error!("promote: {e}");
    }
    if watching(state).await? {
        state.storage().set_alarm(EVERY).await?;
    }
    Response::ok("watching")
}

/// This primary's held digests, as `/healthz/parity` answers them: the Instance's by its object, `MainRun`'s from `state`, as this tick runs in it.
async fn held_here(state: &State, env: &Env, s: &Settings) -> std::result::Result<Value, String> {
    use crate::core::journal;
    let ns = env.durable_object("INSTANCE").map_err(|e| e.to_string())?;
    let stub = ns
        .id_from_name(&s.instance)
        .and_then(|i| i.get_stub())
        .map_err(|e| e.to_string())?;
    let req =
        super::platform::json_request("https://instance/journal/digests", Method::Get, None, None)
            .map_err(|e| e.to_string())?;
    let mut r = platform::call(env, &stub, req)
        .await
        .map_err(|e| e.to_string())?;
    let instance: Value = r.json().await.map_err(|e| e.to_string())?;
    let main: Value = state
        .storage()
        .get(journal::DIGESTS)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(serde_json::json!({ "objects": {
        journal::state_object(&s.instance): { "held": instance },
        journal::main_object(&s.instance): { "held": main },
    }}))
}

/// `MainRun`'s promote routes: `GET /promote` (the promote, the hold and a waiting standby), and the owner's `POST /promote/flip {color}` and `POST /promote/hold {hold}`, which the owner's pages call once a passkey grants them.
///
/// # Errors
///
/// When storage fails, or a body isn't JSON.
pub async fn serve(state: &State, mut req: Request) -> Result<Response> {
    let storage = state.storage();
    match (req.method(), req.path().trim_matches('/')) {
        (Method::Get, "promote") => {
            let promote: Option<Promote> = storage.get(PROMOTE).await?;
            let hold = storage.get::<bool>(HOLD).await?.unwrap_or(true);
            let ready: Option<Checked> = storage.get(READY).await?;
            let flip: Option<String> = storage.get(FLIP).await?;
            Response::from_json(
                &serde_json::json!({ "promote": promote, "hold": hold, "ready": ready, "flip": flip }),
            )
        }
        (Method::Post, "promote/flip") => {
            let body: Value = req.json().await?;
            let Some(color) = body["color"].as_str().filter(|c| !c.is_empty()) else {
                return Response::error("`color` names the colour to flip to", 400);
            };
            if storage.get::<Promote>(PROMOTE).await?.is_some() {
                return Response::error("wait: a promote is in progress", 409);
            }
            storage.put(FLIP, color).await?;
            storage
                .set_alarm(std::time::Duration::from_millis(0))
                .await?;
            Response::ok(format!("flip to {color} starts on the next tick"))
        }
        (Method::Post, "promote/hold") => {
            let body: Value = req.json().await?;
            let hold = body["hold"].as_bool().unwrap_or(true);
            storage.put(HOLD, hold).await?;
            Response::ok(if hold { "held" } else { "released" })
        }
        _ => Response::error("not found", 404),
    }
}
