//! The platform job's third task, `standup <instance>`: an app-shape standalone stood up in the
//! account the owner's one-time token reaches, with its bucket, its Worker, a `STATE_KEY` made
//! here and never kept, and the owner's public keys handed over; see
//! `.claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md`, its 2026-11-15
//! amendments. Its log names no instance, account or host.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::cobot;
use crate::cobot::workers::{self, Found, Spec, Tools};
use crate::core::member::handover::Handover;

/// The app shape's Worker template, from the checkout's root.
pub const TEMPLATE: &str = "catalog/pigeon/kinds/platform/app-worker.yaml";

/// The variable the owner's public keys reach the job in.
pub const HANDOVER_VAR: &str = "HANDOVER";

/// The free plan's compressed Worker size, in KiB.
pub const FREE_LIMIT_KIB: f64 = 3072.0;

/// Whether `argv` asks for a stand-up.
#[must_use]
pub fn asks(argv: &[String]) -> bool {
    argv.first().is_some_and(|v| v == "standup")
}

/// The instance `argv` names: `standup <instance>`.
///
/// # Errors
///
/// A message for any other `argv`.
pub fn parse(argv: &[String]) -> Result<String, String> {
    match argv {
        [verb, name]
            if verb == "standup"
                && !name.is_empty()
                && name.len() <= 40
                && !name.starts_with('-')
                && !name.ends_with('-')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') =>
        {
            Ok(name.clone())
        }
        _ => Err("a stand-up is `standup <instance>`".into()),
    }
}

/// The Worker's script name for `instance`.
#[must_use]
pub fn worker_name(instance: &str) -> String {
    format!("pigeon-{instance}")
}

/// The bucket among `names` an earlier stand-up of `worker` made: `<worker>-<8 hex>`.
#[must_use]
pub fn existing_bucket(names: &[String], worker: &str) -> Option<String> {
    let prefix = format!("{worker}-");
    names
        .iter()
        .find(|n| {
            n.strip_prefix(&prefix)
                .is_some_and(|s| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        .cloned()
}

/// The compressed size wrangler's dry run reports, in KiB: its `gzip: <n> KiB`.
#[must_use]
pub fn gzip_kib(output: &str) -> Option<f64> {
    let at = output.find("gzip:")?;
    let rest = output[at + 5..].trim_start();
    let n: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = n.parse().ok()?;
    let unit = rest[n.len()..].trim_start();
    Some(if unit.starts_with("MiB") {
        value * 1024.0
    } else {
        value
    })
}

/// The outputs its template reads: the bucket it made.
#[must_use]
pub fn outputs(unit: &str, bucket: &str) -> BTreeMap<String, serde_json::Map<String, Value>> {
    let mut o = serde_json::Map::new();
    o.insert("bucket".into(), json!(bucket));
    BTreeMap::from([(unit.to_string(), o)])
}

/// The settings an app-shape stack binds its template with.
#[must_use]
pub fn settings(instance: &str, bucket: &str) -> Value {
    json!({
        "shape": "app", "worker": worker_name(instance), "instance": instance, "domains": [],
        "buckets": [{ "name": bucket }],
    })
}

fn hex(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .map_err(|e| format!("no randomness: {e}"))?;
    Ok(b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    }))
}

/// Stands up `instance` from the checkout at `dir`: its result names only its base URL.
///
/// # Errors
///
/// Saying which step failed; nothing it made is removed, so a retry resumes.
pub fn standup(
    env: &BTreeMap<String, String>,
    dir: &Path,
    instance: &str,
    log: &mut String,
) -> Result<Value, String> {
    let get = |k: &str| env.get(k).filter(|v| !v.is_empty()).cloned();
    let token = get(cobot::cloudflare::TOKEN_VAR)
        .ok_or("the owner's account token didn't reach the job")?;
    let handover = get(HANDOVER_VAR).ok_or("the owner's keys didn't reach the job")?;
    Handover::parse(&handover)?;
    let api = cobot::cloudflare::Api::new(None, token.clone());
    let account = api.account()?;
    let worker = worker_name(instance);
    let sub = api.get(&format!("/accounts/{account}/workers/subdomain"))?;
    let sub = sub["subdomain"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(
            "the account has no workers.dev subdomain yet; the owner chooses one in the dashboard",
        )?
        .to_string();
    let names = cobot::cloudflare::bucket_names(&api, &account)?;
    let found_bucket = existing_bucket(&names, &worker);
    let bucket = match &found_bucket {
        Some(b) => b.clone(),
        None => format!("{worker}-{}", hex(4)?),
    };
    let text =
        std::fs::read_to_string(dir.join(TEMPLATE)).map_err(|e| format!("{TEMPLATE}: {e}"))?;
    let spec: Spec = serde_saphyr::from_str(&text).map_err(|e| format!("{TEMPLATE}: {e}"))?;
    let template = Found {
        unit: format!("instance/{instance}"),
        dir: "catalog/pigeon/kinds/platform".into(),
        spec,
    };
    let found = workers::bind_settings(&template, &template.unit, &settings(instance, &bucket))?;
    let rendered =
        workers::render(&found, &outputs(&template.unit, &bucket)).map_err(|p| p.join("; "))?;
    let out = workers::write(dir, &found, &rendered, "standup").map_err(|e| e.to_string())?;
    let mut tools = Tools::from(|k| std::env::var(k).ok());
    tools.env = vec![
        ("CLOUDFLARE_API_TOKEN".into(), token),
        ("CLOUDFLARE_ACCOUNT_ID".into(), account.clone()),
    ];
    tools.secrets = vec![
        handover.clone(),
        bucket.clone(),
        worker.clone(),
        sub.clone(),
    ];
    let args = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let dry = tools
        .run(
            &tools.wrangler,
            &args(&[
                "deploy",
                "-c",
                "wrangler.jsonc",
                "--dry-run",
                "--outdir",
                "dist",
            ]),
            &out,
            None,
        )
        .map_err(|e| e.to_string())?;
    match gzip_kib(&dry) {
        Some(kib) if kib > FREE_LIMIT_KIB => {
            return Err(format!(
                "the Worker is {kib:.0} KiB compressed, over the free plan's {FREE_LIMIT_KIB:.0} KiB"
            ));
        }
        Some(kib) => {
            let _ = writeln!(log, "the Worker is {kib:.0} KiB compressed");
        }
        None => log.push_str("wrangler didn't report the Worker's size\n"),
    }
    if found_bucket.is_none() {
        cobot::cloudflare::create_bucket(&api, &account, &bucket)?;
        log.push_str("created the bucket\n");
    } else {
        log.push_str("the bucket exists\n");
    }
    workers::wrangler_deploy(&found, &out, &tools).map_err(|e| e.to_string())?;
    log.push_str("deployed the Worker\n");
    let missing = workers::missing_secrets(&found, &tools, &out).map_err(|e| e.to_string())?;
    if missing.iter().any(|m| m == "STATE_KEY") {
        let key = hex(32)?;
        tools.secrets.push(key.clone());
        workers::put_secret(&found, "STATE_KEY", &key, &tools, &out).map_err(|e| e.to_string())?;
        log.push_str("made STATE_KEY in the Worker\n");
    } else {
        log.push_str("STATE_KEY was already set, and is kept\n");
    }
    workers::put_secret(&found, "HANDOVER", &handover, &tools, &out).map_err(|e| e.to_string())?;
    log.push_str("handed over the owner's public keys\n");
    let base = format!("https://{worker}.{sub}.workers.dev");
    check(&base)?;
    log.push_str("the Worker answers /v0/app/* alone\n");
    Ok(json!({ "base_url": base }))
}

/// Whether the stack at `base` serves as an app shape: `/v0/app/whoami` asks for a device, and
/// any other path is not found. A new `workers.dev` host can take a minute to answer.
fn check(base: &str) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let status = |path: &str| {
        agent
            .get(&format!("{base}{path}"))
            .call()
            .map_or(0, |r| r.status().as_u16())
    };
    for _ in 0..12 {
        if status("/v0/app/whoami") == 401 && status("/healthz") == 404 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(10));
    }
    Err("the new Worker didn't answer as an app-shape standalone in two minutes".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_only_a_plain_instance_name() {
        let argv = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(parse(&argv(&["standup", "learner"])).unwrap(), "learner");
        assert!(asks(&argv(&["standup", "x"])));
        for bad in [
            argv(&["standup"]),
            argv(&["standup", "Learner"]),
            argv(&["standup", "a/b"]),
            argv(&["standup", "-x"]),
            argv(&["plan", "learner"]),
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_retry_finds_the_bucket_it_made() {
        let names = vec![
            "pigeon-learner-0a1b2c3d".to_string(),
            "pigeon-learner-old".to_string(),
            "pigeon-platform-state-0a1b2c3d".to_string(),
        ];
        assert_eq!(
            existing_bucket(&names, "pigeon-learner").as_deref(),
            Some("pigeon-learner-0a1b2c3d")
        );
        assert_eq!(existing_bucket(&names, "pigeon-other"), None);
    }

    #[test]
    fn the_dry_run_size_is_read_in_kib() {
        let out = "Total Upload: 9120.55 KiB / gzip: 2803.12 KiB\n--dry-run: exiting now.";
        assert_eq!(gzip_kib(out), Some(2803.12));
        assert_eq!(
            gzip_kib("Total Upload: 12 MiB / gzip: 3.5 MiB"),
            Some(3584.0)
        );
        assert_eq!(gzip_kib("nothing"), None);
    }

    #[test]
    fn the_template_binds_to_an_app_stack() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let text = std::fs::read_to_string(root.join(TEMPLATE)).unwrap();
        let spec: Spec = serde_saphyr::from_str(&text).unwrap();
        let found = Found {
            unit: "instance/learner".into(),
            dir: "catalog/pigeon/kinds/platform".into(),
            spec,
        };
        let bound = workers::bind_settings(
            &found,
            "instance/learner",
            &settings("learner", "pigeon-learner-0a1b2c3d"),
        )
        .unwrap();
        assert_eq!(bound.spec.name, "pigeon-learner");
        assert_eq!(
            bound.spec.bindings.r2["LEARNER"],
            "$output.instance/learner.bucket"
        );
        assert_eq!(bound.spec.bindings.vars["INSTANCE_NAME"], "learner");
        let rendered = workers::render(
            &bound,
            &outputs("instance/learner", "pigeon-learner-0a1b2c3d"),
        )
        .unwrap();
        assert_eq!(
            rendered["r2_buckets"][0]["bucket_name"],
            "pigeon-learner-0a1b2c3d"
        );
        assert_eq!(rendered["workers_dev"], true);
        assert!(rendered.get("routes").is_none());
    }
}
