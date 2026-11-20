//! Minting grants: the token body, the owner's requests, each grant's record and the outcome of
//! its steps; pure, never a secret value; see
//! `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ledger::{ALL_ZONES, Grant, Ledger, Route};

/// The resource prefix of an account-scoped policy.
const ACCOUNT: &str = "com.cloudflare.api.account";

/// The resource prefix of a zone-scoped policy.
const ZONE: &str = "com.cloudflare.api.account.zone";

/// One of the account's permission groups, from `/accounts/<id>/tokens/permission_groups`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Group {
    /// Its ID.
    pub id: String,
    /// Its name, such as `Workers Scripts Write`.
    pub name: String,
    /// The resource scopes it applies to.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// A capability to mint: its catalog name, provider permissions and scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The catalog name, such as `r2.write`.
    pub name: String,
    /// The provider's permission names.
    pub permissions: Vec<String>,
    /// `account` or `zone`.
    pub scope: String,
}

/// The account-token create body for `wanted`, on `account`, each zone in `zones` resolved by
/// `zone_ids`, between `not_before` and `expires_on` (RFC 3339).
///
/// # Errors
///
/// Every permission without exactly one group in its scope, with the names that exist; a zone
/// capability with no zones or an unknown one; a scope grants don't mint.
#[expect(clippy::too_many_arguments, reason = "the token's fields, spelled out")]
pub fn token_body(
    name: &str,
    account: &str,
    wanted: &[Wanted],
    zones: &[String],
    zone_ids: &BTreeMap<String, String>,
    groups: &[Group],
    not_before: &str,
    expires_on: &str,
) -> Result<Value, Vec<String>> {
    let mut problems = Vec::new();
    let mut policies = Vec::new();
    for w in wanted {
        let (scope, resources) = match w.scope.as_str() {
            "account" => (ACCOUNT, json!({ format!("{ACCOUNT}.{account}"): "*" })),
            "zone" => {
                if zones == [ALL_ZONES] {
                    // Every zone in the account; see
                    // .claude/decisions/2026-11-19-security-agents-read-cloudflare-minted.md §2.
                    let all = json!({ format!("{ZONE}.*"): "*" });
                    (ZONE, json!({ format!("{ACCOUNT}.{account}"): all }))
                } else {
                    if zones.is_empty() {
                        problems.push(format!("`{}` is zone-scoped and names no zone", w.name));
                        continue;
                    }
                    let mut r = serde_json::Map::new();
                    for z in zones {
                        match zone_ids.get(z) {
                            Some(id) => {
                                r.insert(format!("{ZONE}.{id}"), json!("*"));
                            }
                            None => problems.push(format!("`{}`'s zone `{z}` is unknown", w.name)),
                        }
                    }
                    (ZONE, Value::Object(r))
                }
            }
            other => {
                problems.push(format!(
                    "`{}` is {other}-scoped, which grants don't mint",
                    w.name
                ));
                continue;
            }
        };
        let mut ids = Vec::new();
        for p in &w.permissions {
            let found: Vec<&Group> = groups
                .iter()
                .filter(|g| g.name == *p && g.scopes.iter().any(|s| s == scope))
                .collect();
            match found.as_slice() {
                [g] => ids.push(json!({ "id": g.id })),
                _ => problems.push(format!(
                    "`{}` needs exactly one `{p}` group in {scope}, found {}; the account has: {}",
                    w.name,
                    found.len(),
                    near(p, groups)
                )),
            }
        }
        policies
            .push(json!({ "effect": "allow", "permission_groups": ids, "resources": resources }));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(json!({
        "name": name,
        "policies": policies,
        "not_before": not_before,
        "expires_on": expires_on,
    }))
}

/// The group names sharing a word with `p`, to show what a wrong name might have meant.
fn near(p: &str, groups: &[Group]) -> String {
    let words: Vec<String> = p
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .map(str::to_lowercase)
        .collect();
    let mut names: Vec<&str> = groups
        .iter()
        .map(|g| g.name.as_str())
        .filter(|n| {
            let n = n.to_lowercase();
            words.iter().any(|w| n.contains(w.as_str()))
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        "no similar name".into()
    } else {
        names.join(", ")
    }
}

/// A capability's provider permissions and scope, as a catalog declares it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Declared {
    /// The provider's permission names.
    pub permissions: Vec<String>,
    /// `account`, `zone` or `bucket`.
    pub scope: String,
}

/// `main`'s ledger as a deploy found it, with each capability it names resolved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Resolved {
    /// The commit deployed.
    pub sha: String,
    /// The ledger.
    pub ledger: Ledger,
    /// Each capability the ledger or a unit names, by catalog name.
    #[serde(default)]
    pub capabilities: BTreeMap<String, Declared>,
    /// Each `pigeon/platform` unit's capability names, by unit, for a `from-unit` mint; see
    /// `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §3.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub units: BTreeMap<String, Vec<String>>,
}

impl Resolved {
    /// What a run of unit `unit` mints: each of its capabilities, with its permissions and scope.
    ///
    /// # Errors
    ///
    /// A unit the deploy reported no set for, or a capability no catalog declares.
    pub fn unit(&self, unit: &str) -> Result<Vec<Wanted>, String> {
        let names = self.units.get(unit).ok_or_else(|| {
            format!(
                "the deploy at {} reported no capabilities for `{unit}`",
                self.sha
            )
        })?;
        names
            .iter()
            .map(|c| {
                let d = self
                    .capabilities
                    .get(c)
                    .ok_or_else(|| format!("`{unit}`'s capability `{c}` is in no catalog"))?;
                Ok(Wanted {
                    name: c.clone(),
                    permissions: d.permissions.clone(),
                    scope: d.scope.clone(),
                })
            })
            .collect()
    }

    /// What grant `name` mints, only the capabilities in `subset`, for one call.
    ///
    /// # Errors
    ///
    /// As [`Self::wanted`], an empty subset, or one naming a capability the grant lacks.
    pub fn wanted_of(&self, name: &str, subset: &[&str]) -> Result<Vec<Wanted>, String> {
        if subset.is_empty() {
            return Err(format!("`{name}`: a call mints at least one capability"));
        }
        let all = self.wanted(name)?;
        if let Some(c) = subset.iter().find(|c| !all.iter().any(|w| w.name == **c)) {
            return Err(format!("`{name}` doesn't grant `{c}`"));
        }
        Ok(all
            .into_iter()
            .filter(|w| subset.contains(&w.name.as_str()))
            .collect())
    }

    /// What grant `name` mints.
    ///
    /// # Errors
    ///
    /// An undeclared grant, one that mints nothing, a capability no catalog declares, or
    /// `from-unit`, which unit runs resolve.
    pub fn wanted(&self, name: &str) -> Result<Vec<Wanted>, String> {
        let g = self
            .ledger
            .grants
            .get(name)
            .ok_or_else(|| format!("`{name}` is not in the ledger at {}", self.sha))?;
        let m = g
            .mint
            .as_ref()
            .ok_or_else(|| format!("`{name}` is not minted"))?;
        let super::ledger::Capabilities::Named(names) = &m.capabilities else {
            return Err(format!(
                "`{name}` takes its unit's capabilities, which a unit run mints"
            ));
        };
        names
            .iter()
            .map(|c| {
                let d = self
                    .capabilities
                    .get(c)
                    .ok_or_else(|| format!("`{name}`'s capability `{c}` is in no catalog"))?;
                Ok(Wanted {
                    name: c.clone(),
                    permissions: d.permissions.clone(),
                    scope: d.scope.clone(),
                })
            })
            .collect()
    }
}

/// What the owner is asked to do to a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Mint or generate it, and put it.
    Request,
    /// Make a new value, put it, verify it, then revoke the old.
    Rotate,
    /// Revoke it and remove its value.
    Revoke,
}

/// A request waiting for the owner's proof, single use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Offer {
    /// Its number, in `/grants/<id>`.
    pub id: u32,
    /// The grant's name.
    pub name: String,
    /// What is asked.
    pub kind: Kind,
    /// For a grant the ledger doesn't declare: its spec, shown on the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Grant>,
    /// The page token's SHA-256 hex, [`crate::approval::grants::key`]; never the token.
    pub token: String,
    /// When it was asked, in Unix seconds.
    pub at: u64,
    /// When the link expires.
    pub expires: u64,
}

/// The requests the Instance keeps, by number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Requests {
    /// The last number given.
    #[serde(default)]
    pub last: u32,
    /// The requests still waiting.
    #[serde(default)]
    pub waiting: Vec<Offer>,
}

impl Requests {
    /// A request of `kind` on `name`, under `token`, until `expires`.
    pub fn offer(
        &mut self,
        name: &str,
        kind: Kind,
        spec: Option<Grant>,
        token: &str,
        now: u64,
        expires: u64,
    ) -> Offer {
        self.waiting.retain(|o| o.expires > now);
        self.last += 1;
        let o = Offer {
            id: self.last,
            name: name.to_string(),
            kind,
            spec,
            token: crate::approval::grants::key(token),
            at: now,
            expires,
        };
        self.waiting.push(o.clone());
        o
    }

    /// The request under `token`, while unexpired.
    ///
    /// # Errors
    ///
    /// When no unexpired request has it.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Offer, String> {
        self.waiting
            .iter()
            .find(|o| !token.is_empty() && o.token == crate::approval::grants::key(token))
            .filter(|o| o.expires > now)
            .ok_or_else(|| "this grant link is not valid, or has expired".into())
    }

    /// Takes the request under `token`, once.
    ///
    /// # Errors
    ///
    /// When [`Requests::peek`] refuses it.
    pub fn redeem(&mut self, token: &str, now: u64) -> Result<Offer, String> {
        let id = self.peek(token, now)?.id;
        let i = self
            .waiting
            .iter()
            .position(|o| o.id == id)
            .ok_or("this grant link is not valid")?;
        Ok(self.waiting.remove(i))
    }
}

/// What is known of one grant: never its value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Record {
    /// The minted token's ID; none for a generated or derived value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    /// Where its value is.
    pub route: Route,
    /// The capabilities minted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// When it was made, in Unix seconds.
    pub made: u64,
    /// When it expires; none for one that doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Who approved it: the owner, or `policy:grants-rotate`.
    pub by: String,
}

/// Each grant's record, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Registry {
    /// The records.
    #[serde(default)]
    pub grants: BTreeMap<String, Record>,
}

/// How a grant's steps ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Put and verified: record it, and revoke the token it replaced, if any.
    Recorded {
        /// The token the new one replaces, to revoke.
        revoke: Option<String>,
    },
    /// Failed after minting: revoke the new token, keep the old, and say why.
    Undo {
        /// The new token, to revoke.
        revoke: String,
        /// Why.
        why: String,
    },
    /// Failed before anything was made.
    Failed {
        /// Why.
        why: String,
    },
}

/// The outcome of minting (`minted`, the new token's ID, or why it failed), then putting and
/// verifying, for a grant whose current token is `old`.
#[must_use]
pub fn settle(
    minted: Result<Option<String>, String>,
    put: Result<(), String>,
    verified: Result<(), String>,
    old: Option<&str>,
) -> Settled {
    let minted = match minted {
        Ok(m) => m,
        Err(why) => return Settled::Failed { why },
    };
    match (put, verified, minted) {
        (Ok(()), Ok(()), _) => Settled::Recorded {
            revoke: old.map(str::to_string),
        },
        (Err(why), _, Some(revoke)) | (Ok(()), Err(why), Some(revoke)) => {
            Settled::Undo { revoke, why }
        }
        (Err(why), _, None) | (Ok(()), Err(why), None) => Settled::Failed { why },
    }
}

/// The ledger's grants past their rotation age at `now`, by their records; a grant never made
/// isn't due, it is requested.
#[must_use]
pub fn due(ledger: &Ledger, registry: &Registry, now: u64) -> Vec<String> {
    ledger
        .grants
        .iter()
        .filter(|(name, g)| {
            registry
                .grants
                .get(*name)
                .is_some_and(|r| super::ledger::due(g, r.made, now))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Capabilities, Mint, Route};

    fn group(id: &str, name: &str, scope: &str) -> Group {
        Group {
            id: id.into(),
            name: name.into(),
            scopes: vec![scope.into()],
        }
    }

    fn groups() -> Vec<Group> {
        vec![
            group("w", "Workers Scripts Write", ACCOUNT),
            group("d", "DNS Write", ZONE),
            group("a", "Containers Write", ACCOUNT),
            group("b", "Containers Write", ACCOUNT),
        ]
    }

    fn wanted(name: &str, permission: &str, scope: &str) -> Wanted {
        Wanted {
            name: name.into(),
            permissions: vec![permission.into()],
            scope: scope.into(),
        }
    }

    fn body(w: &[Wanted], zones: &[&str]) -> Result<Value, Vec<String>> {
        let ids: BTreeMap<String, String> = [("pigeon.dev".to_string(), "z1".to_string())].into();
        let zones: Vec<String> = zones.iter().map(|z| (*z).to_string()).collect();
        token_body("t", "acct", w, &zones, &ids, &groups(), "a", "b")
    }

    #[test]
    fn every_zone_is_one_resource_under_the_account() {
        let b = body(&[wanted("dns.read", "DNS Write", "zone")], &[ALL_ZONES]).expect("a body");
        assert_eq!(
            b["policies"][0]["resources"],
            json!({ "com.cloudflare.api.account.acct": { "com.cloudflare.api.account.zone.*": "*" } })
        );
        assert_eq!(
            b["policies"][0]["permission_groups"],
            json!([{ "id": "d" }])
        );
    }

    #[test]
    fn a_call_mints_only_the_subset_it_names() {
        let declared = |p: &str, scope: &str| Declared {
            permissions: vec![p.into()],
            scope: scope.into(),
        };
        let grant = Grant {
            mint: Some(Mint {
                capabilities: Capabilities::Named(vec!["dns.read".into(), "queues.read".into()]),
                ttl: "5m".into(),
                rotate: None,
                zones: vec![ALL_ZONES.into()],
            }),
            generate: None,
            derive: None,
            route: Route {
                object: Some("Instance".into()),
                ..Route::default()
            },
        };
        let r = Resolved {
            ledger: Ledger {
                grants: [("cloudflare-read".to_string(), grant)].into(),
            },
            capabilities: [
                ("dns.read".to_string(), declared("DNS Read", "zone")),
                (
                    "queues.read".to_string(),
                    declared("Queues Read", "account"),
                ),
            ]
            .into(),
            ..Resolved::default()
        };
        let w = r
            .wanted_of("cloudflare-read", &["queues.read"])
            .expect("a subset");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].permissions, ["Queues Read"]);
        assert!(r.wanted_of("cloudflare-read", &["r2.write"]).is_err());
        assert!(r.wanted_of("cloudflare-read", &[]).is_err());
    }

    #[test]
    fn a_token_body_resolves_each_permission_in_its_scope() {
        let b = body(
            &[
                wanted("workers.write", "Workers Scripts Write", "account"),
                wanted("dns.write", "DNS Write", "zone"),
            ],
            &["pigeon.dev"],
        )
        .expect("a body");
        assert_eq!(
            b["policies"][0]["permission_groups"],
            json!([{ "id": "w" }])
        );
        assert_eq!(
            b["policies"][0]["resources"],
            json!({ "com.cloudflare.api.account.acct": "*" })
        );
        assert_eq!(
            b["policies"][1]["resources"],
            json!({ "com.cloudflare.api.account.zone.z1": "*" })
        );
        assert_eq!(
            (b["not_before"].clone(), b["expires_on"].clone()),
            (json!("a"), json!("b"))
        );
    }

    #[test]
    fn a_wrong_or_ambiguous_name_or_a_missing_zone_mints_nothing_and_says_why() {
        let wrong = body(
            &[wanted("workers.write", "Workers Script Edit", "account")],
            &[],
        )
        .expect_err("no such group");
        assert!(wrong[0].contains("found 0"), "{wrong:?}");
        assert!(
            wrong[0].contains("Workers Scripts Write"),
            "names the near ones: {wrong:?}"
        );
        let two = body(
            &[wanted("containers.write", "Containers Write", "account")],
            &[],
        )
        .expect_err("ambiguous");
        assert!(two[0].contains("found 2"), "{two:?}");
        let scoped = body(&[wanted("dns.write", "DNS Write", "account")], &[]).expect_err("scope");
        assert!(scoped[0].contains("found 0"), "{scoped:?}");
        let zoneless = body(&[wanted("dns.write", "DNS Write", "zone")], &[]).expect_err("zones");
        assert!(zoneless[0].contains("names no zone"), "{zoneless:?}");
        let unknown =
            body(&[wanted("dns.write", "DNS Write", "zone")], &["x.dev"]).expect_err("zone");
        assert!(unknown[0].contains("`x.dev` is unknown"), "{unknown:?}");
        let bucket = body(&[wanted("r2.bucket", "R2 Write", "bucket")], &[]).expect_err("bucket");
        assert!(bucket[0].contains("bucket-scoped"), "{bucket:?}");
    }

    #[test]
    fn a_request_is_single_use_and_expires() {
        let mut r = Requests::default();
        let o = r.offer("vapid", Kind::Request, None, "tok", 10, 100);
        assert_eq!(o.id, 1);
        assert_eq!(
            o.token,
            crate::approval::grants::key("tok"),
            "only the token's hash is kept"
        );
        assert!(
            r.peek(&o.token, 50).is_err(),
            "the stored hash redeems nothing"
        );
        assert!(r.peek("tok", 99).is_ok());
        assert!(r.peek("tok", 100).is_err(), "expired");
        assert!(r.peek("other", 50).is_err());
        assert!(r.peek("", 50).is_err());
        assert_eq!(r.redeem("tok", 50).map(|o| o.name), Ok("vapid".into()));
        assert!(r.redeem("tok", 50).is_err(), "used");
        r.offer("old", Kind::Revoke, None, "t2", 10, 20);
        let next = r.offer("new", Kind::Rotate, None, "t3", 30, 90);
        assert_eq!(next.id, 3);
        assert_eq!(r.waiting.len(), 1, "the expired one is dropped");
    }

    #[test]
    fn each_step_settles_and_a_minted_token_never_outlives_a_failure() {
        let ok = || Ok(());
        let no = || Err("nope".to_string());
        assert_eq!(
            settle(Ok(Some("new".into())), ok(), ok(), Some("old")),
            Settled::Recorded {
                revoke: Some("old".into())
            }
        );
        assert_eq!(
            settle(Ok(Some("new".into())), no(), ok(), Some("old")),
            Settled::Undo {
                revoke: "new".into(),
                why: "nope".into()
            }
        );
        assert_eq!(
            settle(Ok(Some("new".into())), ok(), no(), None),
            Settled::Undo {
                revoke: "new".into(),
                why: "nope".into()
            }
        );
        assert_eq!(
            settle(Ok(None), no(), ok(), None),
            Settled::Failed { why: "nope".into() }
        );
        assert_eq!(
            settle(Err("mint".into()), ok(), ok(), None),
            Settled::Failed { why: "mint".into() }
        );
    }

    #[test]
    fn a_resolved_ledger_names_what_each_grant_mints() {
        let grant = |caps: Capabilities| Grant {
            mint: Some(Mint {
                capabilities: caps,
                ttl: "1h".into(),
                rotate: None,
                zones: Vec::new(),
            }),
            generate: None,
            derive: None,
            route: Route::default(),
        };
        let r = Resolved {
            sha: "abc".into(),
            ledger: Ledger {
                grants: [
                    (
                        "ok".to_string(),
                        grant(Capabilities::Named(vec!["r2.write".into()])),
                    ),
                    (
                        "odd".to_string(),
                        grant(Capabilities::Named(vec!["nope".into()])),
                    ),
                    (
                        "unit".to_string(),
                        grant(Capabilities::Unit("from-unit".into())),
                    ),
                ]
                .into(),
            },
            capabilities: [(
                "r2.write".to_string(),
                Declared {
                    permissions: vec!["Workers R2 Storage Write".into()],
                    scope: "account".into(),
                },
            )]
            .into(),
            units: [
                ("platform/blue".to_string(), vec!["r2.write".to_string()]),
                ("platform/odd".to_string(), vec!["nope".to_string()]),
            ]
            .into(),
        };
        assert_eq!(
            r.unit("platform/blue"),
            Ok(vec![wanted(
                "r2.write",
                "Workers R2 Storage Write",
                "account"
            )])
        );
        assert!(r.unit("platform/odd").is_err_and(|e| e.contains("`nope`")));
        assert!(
            r.unit("platform/green")
                .is_err_and(|e| e.contains("reported no capabilities"))
        );
        assert_eq!(
            r.wanted("ok"),
            Ok(vec![wanted(
                "r2.write",
                "Workers R2 Storage Write",
                "account"
            )])
        );
        assert!(r.wanted("odd").is_err_and(|e| e.contains("`nope`")));
        assert!(r.wanted("unit").is_err_and(|e| e.contains("unit run")));
        assert!(
            r.wanted("absent")
                .is_err_and(|e| e.contains("not in the ledger at abc"))
        );
    }

    #[test]
    fn only_a_made_grant_past_its_rotation_age_is_due() {
        let mint = |rotate: Option<&str>| Grant {
            mint: Some(Mint {
                capabilities: Capabilities::Named(vec!["r2.write".into()]),
                ttl: "30d".into(),
                rotate: rotate.map(str::to_string),
                zones: Vec::new(),
            }),
            generate: None,
            derive: None,
            route: Route {
                job: Some("infra.apply".into()),
                ..Route::default()
            },
        };
        let ledger = Ledger {
            grants: [
                ("a".to_string(), mint(Some("7d"))),
                ("b".to_string(), mint(Some("7d"))),
                ("c".to_string(), mint(None)),
            ]
            .into(),
        };
        let record = |made| Record {
            token_id: Some("t".into()),
            route: Route::default(),
            capabilities: Vec::new(),
            made,
            expires: None,
            by: "owner".into(),
        };
        let registry = Registry {
            grants: [("a".to_string(), record(0)), ("c".to_string(), record(0))].into(),
        };
        assert_eq!(due(&ledger, &registry, 7 * 86_400), ["a"]);
        assert_eq!(due(&ledger, &registry, 1), Vec::<String>::new());
    }
}
