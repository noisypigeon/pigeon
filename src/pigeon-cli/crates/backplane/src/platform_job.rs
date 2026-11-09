//! The platform job: plans or applies one `pigeon/platform` unit on a Runner, with the owner's
//! one-time account token, its state in R2 under `PLATFORM_STATE_KEY`, and existing resources
//! adopted by name; see `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`,
//! step 4's amendment.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use crate::control::{JobSpec, Outcome, Report};

pub use crate::control::PLATFORM;

pub use crate::control::PLATFORM_STATE_KEY_VAR as STATE_KEY_VAR;

/// The state bucket's name, before its random suffix.
pub const STATE_BUCKET: &str = "pigeon-platform-state";

/// The prefix the platform's state lives under in its bucket.
pub const STATE_PREFIX: &str = "platform";

/// What a platform job does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// Plan the unit and store the plan.
    Plan {
        /// The unit, e.g. `platform/blue`.
        unit: String,
    },
    /// Apply a stored plan of the unit.
    Apply {
        /// The unit.
        unit: String,
        /// The stored plan's ID.
        plan: String,
    },
}

impl Task {
    /// The task `argv` names: `plan <unit>` or `apply <unit> <plan>`.
    ///
    /// # Errors
    ///
    /// A message for any other `argv`.
    pub fn parse(argv: &[String]) -> Result<Self, String> {
        match argv {
            [verb, unit] if verb == "plan" && is_unit(unit) => {
                Ok(Self::Plan { unit: unit.clone() })
            }
            [verb, unit, plan] if verb == "apply" && is_unit(unit) && !plan.is_empty() => {
                Ok(Self::Apply {
                    unit: unit.clone(),
                    plan: plan.clone(),
                })
            }
            _ => Err("a platform job is `plan <unit>` or `apply <unit> <plan>`".into()),
        }
    }
}

fn is_unit(u: &str) -> bool {
    u.split_once('/').is_some_and(|(a, n)| {
        a == "platform"
            && !n.is_empty()
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// The state bucket among `names`: the one named `pigeon-platform-state-<suffix>`.
///
/// # Errors
///
/// A message when more than one is.
pub fn state_bucket(names: &[String]) -> Result<Option<String>, String> {
    let prefix = format!("{STATE_BUCKET}-");
    let found: Vec<&String> = names
        .iter()
        .filter(|n| {
            n.strip_prefix(&prefix).is_some_and(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
        })
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).clone())),
        _ => Err(format!(
            "{} buckets are named {STATE_BUCKET}-…",
            found.len()
        )),
    }
}

/// Why a plan with `changes` isn't applied by this job: it destroys something. A stack is
/// removed by `platform.teardown`, never by a plan.
#[must_use]
pub fn destroys(changes: &[cobot::Change]) -> Option<String> {
    let gone: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c.action, cobot::Action::Delete | cobot::Action::Replace))
        .map(|c| format!("{} ({})", c.address, c.action.word()))
        .collect();
    (!gone.is_empty()).then(|| format!("the plan destroys {}", gone.join(", ")))
}

fn random_suffix() -> Result<String, String> {
    let mut b = [0u8; 4];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .map_err(|e| format!("no randomness: {e}"))?;
    Ok(b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}

/// The state bucket on `account`, created with a random suffix if there is none.
fn ensure_bucket(
    api: &cobot::cloudflare::Api,
    account: &str,
    log: &mut String,
) -> Result<String, String> {
    let names = cobot::cloudflare::bucket_names(api, account)?;
    if let Some(b) = state_bucket(&names)? {
        log.push_str("the platform's state bucket exists\n");
        return Ok(b);
    }
    let name = format!("{STATE_BUCKET}-{}", random_suffix()?);
    cobot::cloudflare::create_bucket(api, account, &name)?;
    log.push_str("created the platform's state bucket\n");
    Ok(name)
}

fn changes(c: &[cobot::Change]) -> Value {
    Value::Array(
        c.iter()
            .map(|c| json!({ "address": c.address, "action": c.action.word(), "attributes": c.attributes }))
            .collect(),
    )
}

/// The plan's changes with their values, before and after, as `show` masks them, by
/// resource: the owner reviews a plan before applying it. Output changes are kept as `shown`
/// lists them.
fn with_values(shown: &Value, diff: impl IntoIterator<Item = cobot::AttrChange>) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for d in diff {
        let v = json!({ "attribute": d.attribute, "before": d.before, "after": d.after,
            "replaces": d.replaces });
        match out.iter_mut().find(|c| c["address"] == d.address.as_str()) {
            Some(c) => {
                if let Some(a) = c["attributes"].as_array_mut() {
                    a.push(json!(d.attribute));
                }
                if let Some(a) = c["values"].as_array_mut() {
                    a.push(v);
                }
            }
            None => out.push(json!({ "address": d.address, "action": d.action,
                "attributes": [d.attribute], "values": [v] })),
        }
    }
    out.extend(
        shown
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| {
                c["address"]
                    .as_str()
                    .is_some_and(|a| a.starts_with("output."))
            })
            .cloned(),
    );
    Value::Array(out)
}

/// Runs a platform job from the checkout at `dir`.
#[must_use]
pub fn run(spec: &JobSpec, dir: &Path) -> Report {
    let mut log = String::new();
    let secrets: Vec<String> = spec.env.values().cloned().collect();
    let scrub = |t: &str| {
        secrets
            .iter()
            .filter(|s| s.len() >= 8)
            .fold(t.to_string(), |acc, s| acc.replace(s.as_str(), "…"))
    };
    match work(spec, dir, &mut log) {
        Ok(result) => Report {
            platform: Some(result),
            ..crate::jobs::report(spec, Outcome::Passed, &scrub(&log))
        },
        Err(why) => Report {
            violations: vec![scrub(&why)],
            ..crate::jobs::report(spec, Outcome::Failed, &scrub(&log))
        },
    }
}

fn work(spec: &JobSpec, dir: &Path, log: &mut String) -> Result<Value, String> {
    let task = Task::parse(&spec.argv)?;
    let env = |k: &str| spec.env.get(k).filter(|v| !v.is_empty()).cloned();
    let token = env(cobot::cloudflare::TOKEN_VAR)
        .ok_or("the owner's account token didn't reach the job")?;
    let key = env(STATE_KEY_VAR).ok_or(format!("{STATE_KEY_VAR} isn't set"))?;
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let bucket = ensure_bucket(&api, &account, log)?;
    let vars = [
        (cobot::cloudflare::TOKEN_VAR.to_string(), token.clone()),
        (cobot::PASSPHRASE_VAR.to_string(), key),
        (
            cobot::state::ENDPOINT_VAR.to_string(),
            cobot::sites::endpoint(&account),
        ),
        (
            cobot::state::ACCESS_KEY_VAR.to_string(),
            api.token_id(&account)?,
        ),
        (
            cobot::state::SECRET_KEY_VAR.to_string(),
            cobot::s3::sha256_hex(token.as_bytes()),
        ),
    ];
    let var = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let state = format!("r2://{bucket}/{STATE_PREFIX}");
    let providers = dir.join(".config");
    let failed = |e: cobot::Error| e.to_string();
    let mut s = cobot::session::open(dir, &providers, &state, var).map_err(failed)?;
    let store = cobot::s3::Bucket {
        endpoint: cobot::sites::endpoint(&account),
        name: bucket.clone(),
        creds: cobot::s3::Credentials {
            access_key: api.token_id(&account)?,
            secret_key: cobot::s3::sha256_hex(token.as_bytes()),
        },
    };
    let allow = cobot::providers::Allowlist::load(&providers).map_err(failed)?;
    let (config, found) =
        cobot::providers::fetch_mirror(&store, &allow, dir, cobot::session::now())?;
    let _ = match &config {
        Some(_) => writeln!(log, "providers from the state bucket: {}", found.join(", ")),
        None => writeln!(
            log,
            "no provider packages in the state bucket: installing from the registry"
        ),
    };
    s.ctx.provider_config = config;
    s.ctx.provider_token = true;
    let now = cobot::session::now();
    match task {
        Task::Plan { unit } => {
            s.ctx.adopt = true;
            let units = cobot::select_in(dir, dir, &providers, std::slice::from_ref(&unit))
                .map_err(failed)?;
            let (stored, plans) = cobot::plan_stored(&s.ctx, &units, now).map_err(failed)?;
            s.push().map_err(failed)?;
            let all: Vec<cobot::Change> = plans.iter().flat_map(|p| p.changes.clone()).collect();
            let _ = writeln!(
                log,
                "planned {unit}: {} change(s), plan {}",
                all.len(),
                stored.id
            );
            if let Some(why) = destroys(&all) {
                return Err(why);
            }
            let shown = match cobot::show_stored(&s.ctx, &stored.id, now) {
                Ok(units) => with_values(&changes(&all), units.into_iter().flat_map(|(_, d)| d)),
                Err(e) => {
                    let _ = writeln!(log, "the plan's values couldn't be shown: {e}");
                    changes(&all)
                }
            };
            Ok(json!({ "task": "plan", "unit": unit, "plan": stored.id,
                "expires": stored.expires, "changes": shown }))
        }
        Task::Apply { unit, plan } => {
            let applied = s.apply_stored(&plan, now);
            s.push().map_err(failed)?;
            let applied = applied.map_err(failed)?;
            // Applied is applied: outputs that can't be read are logged, never a failure.
            let outputs = match cobot::outputs(&s.ctx, &unit) {
                Ok(o) => Value::Object(o),
                Err(e) => {
                    let _ = writeln!(log, "the stack's outputs couldn't be read: {e}");
                    Value::Null
                }
            };
            let all: Vec<cobot::Change> = applied.iter().flat_map(|p| p.changes.clone()).collect();
            let _ = writeln!(
                log,
                "applied plan {plan} to {unit}: {} change(s)",
                all.len()
            );
            Ok(
                json!({ "task": "apply", "unit": unit, "plan": plan, "changes": changes(&all),
                "outputs": outputs }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(address: &str, attribute: &str, before: &str, after: &str) -> cobot::AttrChange {
        cobot::AttrChange {
            address: address.into(),
            action: "update".into(),
            attribute: attribute.into(),
            before: before.into(),
            after: after.into(),
            replaces: false,
        }
    }

    #[test]
    fn values_group_by_resource_and_keep_outputs() {
        let shown = json!([
            { "address": "module.t[\"…\"].x.this", "action": "update", "attributes": ["a"] },
            { "address": "output.id", "action": "create", "attributes": [] },
        ]);
        let got = with_values(
            &shown,
            [
                attr("module.t[\"one\"].x.this", "a", "1", "2"),
                attr("module.t[\"two\"].x.this", "a", "1", "3"),
                attr("module.t[\"one\"].x.this", "b", "x", "(sensitive)"),
            ],
        );
        let list = got.as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["attributes"], json!(["a", "b"]));
        assert_eq!(list[0]["values"][1]["after"], "(sensitive)");
        assert_eq!(list[1]["values"][0]["after"], "3");
        assert_eq!(list[2]["address"], "output.id");
    }
}
