//! macOS providers' health on the `Instance` object, beside the CI window, and each provider's
//! minutes this month from D1; see
//! `.claude/decisions/2026-11-12-development-macos-builds-provider-failover.md` §1 and §5.

use serde_json::{Value, json};
use worker::{Env, Response, Result, State};

use super::elicit;
use super::platform::{Settings, d1_rows, now};
use crate::macos::{self, Health, Job, Kind, Outcome, Provider, Usage};

/// The storage key of every provider's health.
const KEY: &str = "macos-health";

/// Every provider's health; empty when the `Instance` doesn't answer, so jobs go on.
pub async fn health(env: &Env, s: &Settings) -> Health {
    match elicit::inbox(env, s, "/macos/health", &json!({})).await {
        Ok(v) => serde_json::from_value(v).unwrap_or_default(),
        Err(e) => {
            worker::console_warn!("macOS health: {e}");
            Health::new()
        }
    }
}

/// Records `what` (`moved`, `ran` or `probing`) for `p`, appending a `macos.provider` event when
/// its health changed.
pub async fn report(env: &Env, s: &Settings, p: Provider, what: &str) {
    let body = json!({ "provider": p, "now": now() });
    match elicit::inbox(env, s, &format!("/macos/{what}"), &body).await {
        Ok(v) => {
            if let Some(said) = v["event"].as_str() {
                let _ =
                    super::events::append(env, s, "macos.provider", None, None, said, false).await;
            }
        }
        Err(e) => worker::console_warn!("macOS health of {}: {e}", p.name()),
    }
}

/// Each provider's minutes since the month began.
///
/// # Errors
///
/// When D1 fails.
pub async fn usage(env: &Env) -> Result<Usage> {
    let rows = d1_rows(env, &macos::usage_since(macos::month_start(now()))).await?;
    Ok(macos::usage_of(&rows))
}

/// Why `p`'s config isn't `main`'s, if it isn't; GitHub's is always `main`'s `ci.yml`.
async fn mismatch(env: &Env, p: Provider) -> std::result::Result<Option<String>, String> {
    match p {
        Provider::Github => Ok(None),
        Provider::Bitrise => super::bitrise::config_mismatch(env).await,
        Provider::Buildkite => super::buildkite::config_mismatch(env).await,
    }
}

/// Moves `job`'s Bitrise or Buildkite attempt on: checks the provider's config and dispatches
/// it at `branch`, or reads how it stands. A GitHub attempt is left to its `Actions`.
pub async fn drive(env: &Env, job: &Job, branch: &str, start_within: u64, at: u64) -> Job {
    let Some(a) = job.current().cloned() else {
        return job.clone();
    };
    if a.provider == Provider::Github {
        return job.clone();
    }
    let Some(id) = a.id.as_deref() else {
        match mismatch(env, a.provider).await {
            Ok(Some(why)) => return macos::ended_as(job, Outcome::Refused, &why, at),
            Err(why) => return macos::dispatch_failed(job, &why, at),
            Ok(None) => {}
        }
        let build = (job.kind == Kind::Archive).then_some(job.number);
        let started = if a.provider == Provider::Bitrise {
            let mut envs = vec![("PIGEON_SHA", job.sha.clone())];
            if let Some(b) = build {
                envs.push(("PIGEON_IOS_BUILD", b.to_string()));
            }
            let workflow = match job.kind {
                Kind::Check => super::bitrise::CHECK_WORKFLOW,
                Kind::Archive => super::bitrise::ARCHIVE_WORKFLOW,
            };
            super::bitrise::trigger(env, workflow, &job.sha, branch, &envs).await
        } else {
            super::buildkite::create_build(env, job.kind.name(), &job.sha, branch, build)
                .await
                .map(|(n, url)| (n.to_string(), url))
        };
        return match started {
            Ok((id, url)) => macos::dispatched(job, &id, url.as_deref()),
            Err(why) => macos::dispatch_failed(job, &why, at),
        };
    };
    let outcome = if a.provider == Provider::Bitrise {
        super::bitrise::build(env, id)
            .await
            .map(|b| macos::bitrise(&b, a.queued_at, at, start_within))
    } else {
        match id.parse::<u64>() {
            Ok(n) => super::buildkite::build_job(env, n)
                .await
                .map(|j| macos::buildkite(&j, a.queued_at, at, start_within)),
            Err(_) => Err(format!("`{id}` is not a Buildkite build number")),
        }
    };
    match outcome {
        Ok(o) => macos::observed(job, o, at),
        Err(why) => {
            worker::console_warn!("macOS {} on {}: {why}", job.kind.name(), a.provider.name());
            job.clone()
        }
    }
}

/// Stops attempt `id` on `p` once its job moved away; GitHub's are cancelled by their caller.
pub async fn abandon(env: &Env, p: Provider, id: &str) {
    let done = match p {
        Provider::Github => Ok(()),
        Provider::Bitrise => super::bitrise::abort(env, id, "moved to another provider").await,
        Provider::Buildkite => match id.parse::<u64>() {
            Ok(n) => super::buildkite::cancel(env, n).await,
            Err(_) => Ok(()),
        },
    };
    if let Err(why) = done {
        worker::console_warn!("abandoning {} {id}: {why}", p.name());
    }
}

/// The archive Bitrise or Buildkite built as `id`.
///
/// # Errors
///
/// When the provider has no such artifact, or a request fails.
pub async fn archive(env: &Env, p: Provider, id: &str) -> std::result::Result<Vec<u8>, String> {
    match p {
        Provider::Bitrise => super::bitrise::artifact(env, id).await,
        Provider::Buildkite => {
            let n: u64 = id
                .parse()
                .map_err(|_| format!("`{id}` is not a Buildkite build number"))?;
            let job = super::buildkite::build_job(env, n).await?;
            let job_id = job["id"].as_str().ok_or("the build has no job")?;
            super::buildkite::artifact(env, n, job_id, crate::iosrelease::ARTIFACT).await
        }
        Provider::Github => Err("GitHub's archive is copied from its run".into()),
    }
}

/// The `Instance` Durable Object's side: `/macos/health`, `moved`, `ran` and `probing`.
///
/// # Errors
///
/// When storage fails.
pub async fn serve(state: &State, path: &str, body: &Value) -> Result<Response> {
    let storage = state.storage();
    let mut all: Health = storage.get(KEY).await?.unwrap_or_default();
    if path == "/macos/health" {
        return Response::from_json(&all);
    }
    let Some(p) = body["provider"].as_str().and_then(Provider::parse) else {
        return Response::error("no provider", 400);
    };
    let t = body["now"].as_u64().unwrap_or_else(now);
    let before = all.get(&p).cloned().unwrap_or_default();
    let after = match path {
        "/macos/moved" => macos::moved_away(&before, t),
        "/macos/ran" => macos::ran(&before),
        "/macos/probing" => macos::probing(&before),
        _ => return Response::error("not found", 404),
    };
    let event = macos::health_event(p, &before, &after);
    all.insert(p, after);
    storage.put(KEY, &all).await?;
    Response::from_json(&json!({ "health": all, "event": event }))
}
