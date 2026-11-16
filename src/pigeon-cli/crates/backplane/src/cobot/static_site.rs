//! Static sites on disk: the workloads a zone serves and their bucket and token checks, from
//! core's `static_site`; see `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::cobot::apex::{self, Site};

pub use crate::core::infra::static_site::*;

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
        files: apex::disk(),
    }
}

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
            format!(
                "workloads/{zone}/{w}/{}",
                crate::core::infra::config::CONFIG
            ),
            out,
        );
    }
}
