//! Static sites on R2: a workload's bucket and custom domains, its zone's
//! rulesets, and their tokens; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::apex::{self, Site};
use crate::generations::Day;

/// A workload's handoff kind.
pub const HANDOFF: &str = "workload-token";
/// A workload's handoff keys.
pub const HANDOFF_KEYS: [&str; 3] = ["api_token", "zone_id", "account_id"];
/// The bucket expander.
pub const BUCKET: &str = "site-bucket";
/// The custom-domain expander.
pub const DOMAINS: &str = "site-domains";
/// The apex's ruleset expander.
pub const RULESETS: &str = "zone-rulesets";
/// The bucket item's key, which each domain names.
pub const BUCKET_KEY: &str = "site";
/// An inert account id a plan uses while the handoff is pending.
pub const PLACEHOLDER_ACCOUNT_ID: &str = "00000000000000000000000000000000";
/// The zone permissions an apex token adds for its static sites' rulesets.
pub const APEX_RULE_PERMISSIONS: [&str; 2] =
    ["Zone Transform Rules Write", "Dynamic URL Redirects Write"];
/// A site token's account permission.
pub const R2_WRITE: &str = "Workers R2 Storage Write";
/// A site token's zone permissions, beside [`apex::DNS_WRITE`].
pub const ZONE_READ: &str = "Zone Read";

/// The zone and workload of a workload path, `workloads/<zone>/<workload>`.
#[must_use]
pub fn zone_workload(path: &str) -> Option<(&str, &str)> {
    let mut parts = path.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("workloads"), Some(zone), Some(w), None)
            if w != apex::APEX && apex::is_zone(zone) && apex::is_name(w) =>
        {
            Some((zone, w))
        }
        _ => None,
    }
}

/// Whether a workload's config enables a static site.
#[must_use]
pub fn is_static(config: &Value) -> bool {
    config.get("static_site").is_some()
}

/// A site's settings: a site unit's own config, or a workload's `static_site`.
#[must_use]
pub fn settings<'a>(config: &'a Value, site: &Site) -> Option<&'a Value> {
    if site.manifest.is_some() {
        site.workload.as_ref().map(|_| config)
    } else {
        config.get("static_site")
    }
}

/// The token `infra/cloudflare` mints for a static site.
#[must_use]
pub fn token_name(zone: &str, workload: &str) -> String {
    format!("site-{zone}-{workload}")
}

/// A static site's bucket name: the workload, then the zone with dots as dashes.
#[must_use]
pub fn bucket_name(zone: &str, workload: &str) -> String {
    format!("{workload}-{}", zone.replace('.', "-"))
}

/// The static-site workloads in a zone, by name.
#[must_use]
pub fn sites(root: &Path, zone: &str) -> Vec<String> {
    sites_of(&on_disk(root, zone), zone)
}

/// A [`Site`] reading `workloads/` under `root`.
fn on_disk(root: &Path, zone: &str) -> Site {
    Site {
        root: root.to_path_buf(),
        zone: zone.to_string(),
        workload: None,
        manifest: None,
    }
}

/// The static-site workloads `site` sees in a zone, by name.
fn sites_of(site: &Site, zone: &str) -> Vec<String> {
    site.workloads(zone)
        .into_iter()
        .filter(|(w, config)| *config && site.config_of(zone, w).is_some_and(|c| is_static(&c)))
        .map(|(w, _)| w)
        .collect()
}

/// The hosts the apex assigns to each static site in a zone.
fn site_hosts(config: &Value, site: &Site, zone: &str) -> BTreeMap<String, Vec<String>> {
    let units = site.workloads(zone);
    let wanted = sites_of(site, zone);
    apex::assignments(config, &units, zone)
        .into_iter()
        .filter(|(w, _)| wanted.contains(w))
        .collect()
}

/// The hosts `site`'s zone assigns workload `w`.
#[must_use]
pub fn hosts(site: &Site, w: &str) -> Vec<String> {
    apex::assignments(
        &site.apex_config(&site.zone),
        &site.workloads(&site.zone),
        &site.zone,
    )
    .remove(w)
    .unwrap_or_default()
}

/// `site-bucket`: one item, keyed [`BUCKET_KEY`], when the workload is a static site.
#[must_use]
pub fn bucket(config: &Value, site: &Site) -> Map<String, Value> {
    let mut out = Map::new();
    if let (Some(w), Some(s)) = (&site.workload, settings(config, site)) {
        let class = s["storage_class"].as_str().unwrap_or("Standard");
        out.insert(
            BUCKET_KEY.into(),
            json!({ "name": bucket_name(&site.zone, w), "storage_class": class }),
        );
    }
    out
}

/// `site-domains`: one custom domain per host the apex assigns, keyed by its label.
#[must_use]
pub fn domains(config: &Value, site: &Site) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::new();
    let (Some(w), Some(_)) = (&site.workload, settings(config, site)) else {
        return (out, Vec::new());
    };
    let hosts = hosts(site, w);
    if hosts.is_empty() {
        let why = if site.manifest.is_some() {
            "hosts: a static site needs a host".to_string()
        } else {
            format!(
                "static_site: workloads/{}/{} assigns this workload no host",
                site.zone,
                apex::APEX
            )
        };
        return (out, vec![why]);
    }
    for h in hosts {
        out.insert(
            apex::label(&h),
            json!({ "bucket": BUCKET_KEY, "domain": apex::fqdn(&h, &site.zone) }),
        );
    }
    (out, Vec::new())
}

/// `zone-rulesets`: each zone's `sites-root` and `sites-www`, keyed
/// `<zone>:<name>`, for its static sites; none in a zone without one. `config` is composed.
#[must_use]
pub fn rulesets(config: &Value, site: &Site) -> Map<String, Value> {
    let mut out = Map::new();
    for zone in apex::zones(config) {
        let hosts = site_hosts(&config["zones"][&zone], site, &zone);
        let mut rewrites = Vec::new();
        let mut redirects = Vec::new();
        for (w, list) in &hosts {
            for h in list {
                let fqdn = apex::fqdn(h, &zone);
                rewrites.push(json!({
                    "description": format!("{w}: {fqdn}/ serves /index.html"),
                    "expression": format!("(http.host eq \"{fqdn}\" and http.request.uri.path eq \"/\")"),
                    "action": "rewrite",
                    "action_parameters": { "uri": { "path": { "value": "/index.html" } } }
                }));
            }
            if list.iter().any(|h| h == "@") && list.iter().any(|h| h == "www") {
                redirects.push(json!({
                    "description": format!("{w}: www.{zone} redirects to {zone}"),
                    "expression": format!("(http.host eq \"www.{zone}\")"),
                    "action": "redirect",
                    "action_parameters": { "from_value": {
                        "status_code": 301,
                        "preserve_query_string": true,
                        "target_url": { "expression": format!("concat(\"https://{zone}\", http.request.uri.path)") }
                    } }
                }));
            }
        }
        for (name, phase, rules) in [
            ("sites-root", "http_request_transform", rewrites),
            ("sites-www", "http_request_dynamic_redirect", redirects),
        ] {
            if !rules.is_empty() {
                out.insert(
                    format!("{zone}:{name}"),
                    json!({ "zone": zone, "name": name, "phase": phase, "rules": rules }),
                );
            }
        }
    }
    out
}

/// R2's longest bucket name.
pub const BUCKET_MAX: usize = 63;

/// Checks that no two static sites share a bucket name, and that each fits R2.
pub fn check_buckets(root: &Path, zones: &[&str], out: &mut Vec<(String, String)>) {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for zone in zones {
        for w in sites(root, zone) {
            let name = bucket_name(zone, &w);
            let at = format!("workloads/{zone}/{w}");
            if name.len() > BUCKET_MAX {
                out.push((
                    at.clone(),
                    format!("bucket `{name}` is longer than R2's {BUCKET_MAX} characters"),
                ));
            }
            if let Some(first) = seen.insert(name.clone(), at.clone()) {
                out.push((at, format!("bucket `{name}` is also {first}'s")));
            }
        }
    }
}

/// Checks that each static site in `zone` has exactly its `site-<zone>-<workload>` token.
pub fn check_tokens(root: &Path, cf: &Value, zone: &str, out: &mut Vec<(String, String)>) {
    for w in sites(root, zone) {
        let want = [
            (R2_WRITE.to_string(), true, Vec::new()),
            (ZONE_READ.to_string(), false, vec![zone.to_string()]),
            (apex::DNS_WRITE.to_string(), false, vec![zone.to_string()]),
        ];
        apex::check_token(
            cf,
            &token_name(zone, &w),
            &want,
            format!("workloads/{zone}/{w}/{}", crate::config::CONFIG),
            out,
        );
    }
}

/// A static site's handed-off values from `infra/cloudflare`'s `tofu show -json`
/// state; a key is missing until state holds it.
#[must_use]
pub fn handoff(
    state_json: &str,
    zone: &str,
    workload: &str,
    today: Day,
) -> BTreeMap<String, String> {
    let state = serde_json::from_str::<Value>(state_json).unwrap_or(Value::Null);
    let outputs = &state["values"]["outputs"];
    let mut values = BTreeMap::new();
    if let Some(id) = outputs["zone_ids"]["value"][zone].as_str() {
        values.insert("zone_id".to_string(), id.to_string());
    }
    if let Some(id) = outputs["account_id"]["value"].as_str() {
        values.insert("account_id".to_string(), id.to_string());
    }
    if let Some((_, v)) = apex::live_token(outputs, "tokens", &token_name(zone, workload), today) {
        values.insert("api_token".to_string(), v);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::{bucket_name, handoff, token_name, zone_workload};

    #[test]
    fn names_follow_the_zone_and_workload() {
        assert_eq!(
            zone_workload("workloads/noisypigeon.com/blog"),
            Some(("noisypigeon.com", "blog"))
        );
        assert_eq!(zone_workload("workloads/noisypigeon.com/apex"), None);
        assert_eq!(zone_workload("workloads/cloudflare/blog"), None);
        assert_eq!(zone_workload("workloads/noisypigeon.com/blog/x"), None);
        assert_eq!(
            bucket_name("noisypigeon.com", "blog"),
            "blog-noisypigeon-com"
        );
        assert_eq!(
            token_name("noisypigeon.com", "blog"),
            "site-noisypigeon.com-blog"
        );
    }

    #[test]
    fn a_site_handoff_takes_its_own_token() {
        let d = |s| crate::generations::day(s).expect("day");
        let state = serde_json::json!({ "values": { "outputs": {
            "zone_ids": { "value": { "z.com": "zid" } },
            "account_id": { "value": "acct" },
            "tokens": { "value": {
                "site-z.com-blog@2026-11-02..2027-02-07": "site",
                "apex-z.com@2026-11-02..2027-02-07": "apex"
            } }
        } } })
        .to_string();
        let got = handoff(&state, "z.com", "blog", d("2026-11-05"));
        assert_eq!(got["api_token"], "site");
        assert_eq!(got["zone_id"], "zid");
        assert_eq!(got["account_id"], "acct");
        assert!(!handoff(&state, "z.com", "docs", d("2026-11-05")).contains_key("api_token"));
    }
}
