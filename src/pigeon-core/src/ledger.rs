//! The grants ledger, `.config/grants.yaml`: each secret the control plane mints, generates or
//! derives, by name, and where it is put; never a value; see
//! `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The ledger's path under the root.
pub const LEDGER: &str = ".config/grants.yaml";

/// The capabilities a `from-unit` mint takes from its unit's `config.yaml`.
pub const FROM_UNIT: &str = "from-unit";

/// The job a platform run's token is routed to; see
/// `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
pub const PLATFORM_JOB: &str = "platform.plan";

/// The ledger: each grant by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    /// The grants, by name.
    #[serde(default)]
    pub grants: BTreeMap<String, Grant>,
}

/// One grant: exactly one source, and its route.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// A Cloudflare account token, minted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mint: Option<Mint>,
    /// A key made in the Worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generate: Option<Generate>,
    /// A value computed from another grant or from state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derive: Option<Derive>,
    /// Where the value is put.
    pub route: Route,
}

/// A minted token's capabilities, lifetime and rotation age.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mint {
    /// Capability names from `catalog/*/capabilities.yaml`, or [`FROM_UNIT`].
    pub capabilities: Capabilities,
    /// How long it lives: `15m`, `1h`, `30d`.
    pub ttl: String,
    /// How old it grows before the hourly cron re-mints it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<String>,
    /// The zones a zone-scoped capability is minted on, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<String>,
}

/// A mint's capabilities: named, or its unit's.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Capabilities {
    /// Named capabilities.
    Named(Vec<String>),
    /// [`FROM_UNIT`].
    Unit(String),
}

/// A key generated in the Worker.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    /// Its kind.
    pub kind: KeyKind,
}

/// What a generated key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    /// 32 random bytes.
    Random32,
    /// A P-256 key pair.
    P256,
    /// An HMAC key.
    Hmac,
    /// An age identity.
    Age,
}

/// A derived value's source: another grant, or `state:<unit>/<output>`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Derive {
    /// What it is computed from.
    pub from: String,
}

/// Where a value goes: exactly one of a Worker secret, a Secrets Store secret, or one job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// A Worker, by script name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// A Secrets Store, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// A job, by operation name; its environment only, revoked when it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// A Durable Object class, by name: minted by the object at run time, held in its memory
    /// only; see `.claude/decisions/2026-11-16-security-ai-gateway-claude-api.md`, the Run token
    /// amendment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// The secret's name, for a Worker or a store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// `text` (`15m`, `1h`, `30d`) in seconds; none when malformed or zero.
#[must_use]
pub fn seconds(text: &str) -> Option<u64> {
    let (n, unit) = text.split_at(text.len().checked_sub(1)?);
    let n: u64 = n.parse().ok().filter(|n| *n > 0)?;
    let per = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return None,
    };
    n.checked_mul(per)
}

fn slug(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn class(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn key(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Every problem with `ledger`, given the capability names the catalogs declare.
#[must_use]
pub fn check(ledger: &Ledger, known: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, g) in &ledger.grants {
        let mut say = |p: String| problems.push(format!("{LEDGER}: grant `{name}` {p}"));
        if !slug(name) {
            say("is not named [a-z0-9-]+".into());
        }
        let sources = [g.mint.is_some(), g.generate.is_some(), g.derive.is_some()]
            .into_iter()
            .filter(|s| *s)
            .count();
        if sources != 1 {
            say(format!(
                "has {sources} sources; it takes exactly one of mint, generate or derive"
            ));
        }
        let r = &g.route;
        let places = [
            r.worker.is_some(),
            r.store.is_some(),
            r.job.is_some(),
            r.object.is_some(),
        ]
        .into_iter()
        .filter(|p| *p)
        .count();
        if places != 1 {
            say(format!(
                "routes to {places} places; it takes exactly one of worker, store, job or object"
            ));
        }
        match (&r.job, &r.object, &r.key) {
            (Some(_), _, Some(_)) => {
                say("names a key for a job, whose environment has none".into());
            }
            (_, Some(_), Some(_)) => say("names a key for an object, which keeps none".into()),
            (None, None, None) => say("routes to a Worker or store with no key".into()),
            (None, None, Some(k)) if !key(k) => say(format!("key `{k}` is not [A-Z][A-Z0-9_]*")),
            _ => {}
        }
        if r.job.is_some() && g.mint.is_none() {
            say("routes to a job, which takes only a minted token".into());
        }
        if let Some(o) = &r.object {
            if !class(o) {
                say(format!(
                    "object `{o}` is not a class name, [A-Z][A-Za-z0-9]*"
                ));
            }
            if g.mint.is_none() {
                say("routes to an object, which takes only a minted token".into());
            }
            if g.mint.as_ref().is_some_and(|m| m.rotate.is_some()) {
                say("routes to an object, which mints its own; it takes no rotate".into());
            }
        }
        if let Some(d) = &g.derive
            && d.from.trim().is_empty()
        {
            say("derives from nothing".into());
        }
        let Some(m) = &g.mint else {
            continue;
        };
        let ttl = seconds(&m.ttl);
        if ttl.is_none() {
            say(format!("ttl `{}` is not like 15m, 1h or 30d", m.ttl));
        }
        if let Some(rotate) = &m.rotate {
            match (seconds(rotate), ttl) {
                (None, _) => say(format!("rotate `{rotate}` is not like 15m, 1h or 30d")),
                (Some(r), Some(t)) if r >= t => {
                    say(format!("rotates at {rotate}, not before its ttl {}", m.ttl));
                }
                _ => {}
            }
        }
        match &m.capabilities {
            Capabilities::Unit(u) if u != FROM_UNIT => {
                say(format!(
                    "capabilities `{u}` is neither a list nor `{FROM_UNIT}`"
                ));
            }
            Capabilities::Named(c) if c.is_empty() => say("mints no capability".into()),
            Capabilities::Named(c) => {
                for unknown in c.iter().filter(|c| !known.contains(*c)) {
                    say(format!(
                        "names capability `{unknown}`, which no catalog declares"
                    ));
                }
            }
            Capabilities::Unit(_) => {}
        }
        if r.job.as_deref() == Some(PLATFORM_JOB) {
            platform_run(m, ttl, &mut say);
        }
    }
    problems
}

/// A platform run's mint: its unit's capabilities, for one run; see
/// `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
fn platform_run(m: &Mint, ttl: Option<u64>, say: &mut impl FnMut(String)) {
    if m.capabilities != Capabilities::Unit(FROM_UNIT.into()) {
        say(format!(
            "routes to `{PLATFORM_JOB}`, which takes `{FROM_UNIT}`"
        ));
    }
    if ttl != Some(crate::platform::platforms::LIFETIME) {
        say(format!(
            "routes to `{PLATFORM_JOB}`, whose token lives one run: ttl 24h"
        ));
    }
    if m.rotate.is_some() {
        say(format!(
            "routes to `{PLATFORM_JOB}`, whose token lives one run; it takes no rotate"
        ));
    }
}

/// Whether `grant`, last minted at `last`, is past its rotation age at `now`.
#[must_use]
pub fn due(grant: &Grant, last: u64, now: u64) -> bool {
    grant
        .mint
        .as_ref()
        .and_then(|m| m.rotate.as_deref())
        .and_then(seconds)
        .is_some_and(|age| now.saturating_sub(last) >= age)
}

/// The ledger in `text`, checked against `catalogs`: none when the instance has no ledger
/// (`text` is `None`).
///
/// # Errors
///
/// The ledger's problems, when it doesn't parse or doesn't check.
pub fn parse(
    text: Option<&str>,
    catalogs: &[super::infra::capabilities::Catalog],
) -> Result<Option<Ledger>, Vec<String>> {
    let Some(text) = text else {
        return Ok(None);
    };
    let ledger: Ledger =
        serde_saphyr::from_str(text).map_err(|e| vec![format!("{LEDGER}: {e}")])?;
    let known: BTreeSet<String> = catalogs
        .iter()
        .flat_map(|c| c.capabilities.keys().cloned())
        .collect();
    let problems = check(&ledger, &known);
    if problems.is_empty() {
        Ok(Some(ledger))
    } else {
        Err(problems)
    }
}

/// `ledger` with each capability it or one of `units` (each `pigeon/platform` unit's capability
/// names, by unit) names resolved from `catalogs`, as deployed at `sha`; see
/// `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
#[must_use]
pub fn resolved(
    ledger: Ledger,
    catalogs: Vec<super::infra::capabilities::Catalog>,
    sha: &str,
    units: BTreeMap<String, Vec<String>>,
) -> super::minting::Resolved {
    let named: BTreeSet<&String> = ledger
        .grants
        .values()
        .filter_map(|g| match g.mint.as_ref().map(|m| &m.capabilities) {
            Some(Capabilities::Named(c)) => Some(c),
            _ => None,
        })
        .flatten()
        .chain(units.values().flatten())
        .collect();
    let capabilities = catalogs
        .into_iter()
        .flat_map(|c| c.capabilities)
        .filter(|(name, _)| named.contains(name))
        .map(|(name, c)| {
            let declared = super::minting::Declared {
                permissions: c.permissions,
                scope: c.scope,
            };
            (name, declared)
        })
        .collect();
    super::minting::Resolved {
        sha: sha.to_string(),
        ledger,
        capabilities,
        units,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> BTreeSet<String> {
        ["r2.write", "d1.write"].map(String::from).into()
    }

    fn worker(key: &str) -> Route {
        Route {
            worker: Some("pigeon-control".into()),
            key: Some(key.into()),
            ..Route::default()
        }
    }

    fn mint(caps: &[&str], ttl: &str, rotate: Option<&str>) -> Grant {
        Grant {
            mint: Some(Mint {
                capabilities: Capabilities::Named(caps.iter().map(|c| (*c).to_string()).collect()),
                ttl: ttl.into(),
                rotate: rotate.map(str::to_string),
                zones: Vec::new(),
            }),
            generate: None,
            derive: None,
            route: worker("DEPLOY_TOKEN"),
        }
    }

    fn one(name: &str, g: Grant) -> Vec<String> {
        check(
            &Ledger {
                grants: [(name.to_string(), g)].into(),
            },
            &known(),
        )
    }

    #[test]
    fn a_mint_a_generated_key_and_a_derived_value_check_clean() {
        assert_eq!(
            one("deploy", mint(&["r2.write"], "30d", Some("7d"))),
            Vec::<String>::new()
        );
        let generated = Grant {
            mint: None,
            generate: Some(Generate {
                kind: KeyKind::P256,
            }),
            derive: None,
            route: worker("PUSH_VAPID_KEY"),
        };
        assert_eq!(one("vapid", generated), Vec::<String>::new());
        let derived = Grant {
            mint: None,
            generate: None,
            derive: Some(Derive {
                from: "state:infra/pigeon/outputs".into(),
            }),
            route: Route {
                store: Some("pigeon".into()),
                key: Some("DEPLOY_OUTPUTS".into()),
                ..Route::default()
            },
        };
        assert_eq!(one("outputs", derived), Vec::<String>::new());
        let mut job = mint(&[], "1h", None);
        job.mint.as_mut().expect("mint").capabilities = Capabilities::Unit(FROM_UNIT.into());
        job.route = Route {
            job: Some("infra.apply".into()),
            ..Route::default()
        };
        assert_eq!(one("infra-run", job), Vec::<String>::new());
    }

    // See `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
    #[test]
    fn a_platform_run_takes_its_units_capabilities_for_one_run() {
        let run = |caps: Capabilities, ttl: &str, rotate: Option<&str>| {
            let mut g = mint(&["r2.write"], ttl, rotate);
            g.mint.as_mut().expect("mint").capabilities = caps;
            g.route = Route {
                job: Some(PLATFORM_JOB.into()),
                ..Route::default()
            };
            g
        };
        let unit = || Capabilities::Unit(FROM_UNIT.into());
        assert_eq!(
            one("platform-run", run(unit(), "24h", None)),
            Vec::<String>::new()
        );
        let named = one(
            "platform-run",
            run(Capabilities::Named(vec!["r2.write".into()]), "24h", None),
        );
        assert!(
            named.iter().any(|p| p.contains("takes `from-unit`")),
            "{named:?}"
        );
        let long = one("platform-run", run(unit(), "30d", None));
        assert!(long.iter().any(|p| p.contains("ttl 24h")), "{long:?}");
        let rotating = one("platform-run", run(unit(), "24h", Some("1h")));
        assert!(
            rotating.iter().any(|p| p.contains("takes no rotate")),
            "{rotating:?}"
        );
    }

    fn object(g: Grant, class: &str) -> Grant {
        Grant {
            route: Route {
                object: Some(class.into()),
                ..Route::default()
            },
            ..g
        }
    }

    #[test]
    fn a_minted_token_routes_to_an_object() {
        let g = object(mint(&["r2.write"], "1h", None), "Pool");
        assert_eq!(one("aigateway", g), Vec::<String>::new());
    }

    #[test]
    fn an_object_route_is_refused_a_key_a_rotate_a_bad_class_or_a_second_place() {
        let has = |p: &[String], s: &str| p.iter().any(|p| p.contains(s));
        let mut keyed = object(mint(&["r2.write"], "1h", None), "Pool");
        keyed.route.key = Some("X".into());
        assert!(has(&one("x", keyed), "a key for an object"));
        let rotated = object(mint(&["r2.write"], "1h", Some("50m")), "Pool");
        assert!(has(&one("x", rotated), "takes no rotate"));
        let lower = object(mint(&["r2.write"], "1h", None), "pool");
        assert!(has(&one("x", lower), "not a class name"));
        let mut two = object(mint(&["r2.write"], "1h", None), "Pool");
        two.route.job = Some("infra.apply".into());
        assert!(has(&one("x", two), "routes to 2 places"));
        let generated = Grant {
            mint: None,
            generate: Some(Generate {
                kind: KeyKind::Random32,
            }),
            ..object(mint(&[], "1h", None), "Pool")
        };
        assert!(has(&one("x", generated), "only a minted token"));
    }

    #[test]
    fn each_malformed_grant_is_named() {
        let has = |p: &[String], s: &str| p.iter().any(|p| p.contains(s));
        assert!(has(
            &one("Bad", mint(&["r2.write"], "1h", None)),
            "[a-z0-9-]+"
        ));
        assert!(has(&one("x", mint(&["r2.write"], "1y", None)), "ttl `1y`"));
        assert!(has(&one("x", mint(&["r2.write"], "0h", None)), "ttl `0h`"));
        assert!(has(
            &one("x", mint(&["r2.write"], "1d", Some("1d"))),
            "not before its ttl"
        ));
        assert!(has(
            &one("x", mint(&["r2.write"], "1d", Some("soon"))),
            "rotate `soon`"
        ));
        assert!(has(
            &one("x", mint(&["zone.own"], "1h", None)),
            "`zone.own`"
        ));
        assert!(has(&one("x", mint(&[], "1h", None)), "mints no capability"));
        let mut g = mint(&["r2.write"], "1h", None);
        g.mint.as_mut().expect("mint").capabilities = Capabilities::Unit("everything".into());
        assert!(has(&one("x", g), "neither a list"));

        let mut two = mint(&["r2.write"], "1h", None);
        two.generate = Some(Generate {
            kind: KeyKind::Random32,
        });
        assert!(has(&one("x", two), "2 sources"));
        let mut none = mint(&["r2.write"], "1h", None);
        none.mint = None;
        assert!(has(&one("x", none), "0 sources"));

        let mut both = mint(&["r2.write"], "1h", None);
        both.route.store = Some("pigeon".into());
        assert!(has(&one("x", both), "2 places"));
        let mut lower = mint(&["r2.write"], "1h", None);
        lower.route.key = Some("deploy_token".into());
        assert!(has(&one("x", lower), "key `deploy_token`"));
        let mut keyless = mint(&["r2.write"], "1h", None);
        keyless.route.key = None;
        assert!(has(&one("x", keyless), "with no key"));
        let job_key = Grant {
            route: Route {
                job: Some("infra.apply".into()),
                key: Some("TOKEN".into()),
                ..Route::default()
            },
            ..mint(&["r2.write"], "1h", None)
        };
        assert!(has(&one("x", job_key), "a key for a job"));
        let generated_job = Grant {
            mint: None,
            generate: Some(Generate {
                kind: KeyKind::Hmac,
            }),
            derive: None,
            route: Route {
                job: Some("infra.apply".into()),
                ..Route::default()
            },
        };
        assert!(has(&one("x", generated_job), "only a minted token"));
    }

    #[test]
    fn a_typo_is_refused_not_ignored() {
        let text = r#"{"grants": {"x": {"mint": {"capabilities": ["r2.write"], "ttl": "1h", "scope": "all"}, "route": {"job": "infra.apply"}}}}"#;
        assert!(serde_json::from_str::<Ledger>(text).is_err());
    }

    #[test]
    fn a_mint_is_due_past_its_rotation_age_only() {
        let g = mint(&["r2.write"], "30d", Some("7d"));
        assert!(!due(&g, 0, 7 * 86_400 - 1));
        assert!(due(&g, 0, 7 * 86_400));
        assert!(!due(&mint(&["r2.write"], "30d", None), 0, u64::MAX));
        assert_eq!(seconds("15m"), Some(900));
        assert_eq!(seconds(""), None);
    }
}
