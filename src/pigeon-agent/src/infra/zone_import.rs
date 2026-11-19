//! Importing a zone's live records: the API read and the records the units already render; see
//! `.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md` §5.

use std::path::Path;

use serde_json::Value;

pub use crate::core::infra::zone_import::*;

/// Zone `zone`'s DNS records from the API, and what it holds outside DNS, by kind.
///
/// # Errors
///
/// A message when the zone or its records can't be read.
pub fn fetch(
    api: &crate::workers::cloudflare::Api,
    zone: &str,
) -> Result<(Vec<Value>, Vec<Unmanaged>), String> {
    let id = api.zone_id(zone)?;
    let records = api.get_all(&format!("/zones/{id}/dns_records"))?;
    let mut outside = Vec::new();
    let mut note = |kind: &str, found: Result<bool, String>| match found {
        Ok(true) => outside.push(Unmanaged {
            kind: kind.into(),
            name: None,
        }),
        Ok(false) => {}
        Err(_) => outside.push(Unmanaged {
            kind: format!("{kind} (unread: the token can't read it)"),
            name: None,
        }),
    };
    note(
        "DNSSEC",
        api.get(&format!("/zones/{id}/dnssec"))
            .map(|v| v["status"] != "disabled"),
    );
    note(
        "Workers routes",
        api.get(&format!("/zones/{id}/workers/routes"))
            .map(|v| v.as_array().is_some_and(|a| !a.is_empty())),
    );
    note(
        "rulesets",
        api.get(&format!("/zones/{id}/rulesets")).map(|v| {
            v.as_array()
                .is_some_and(|a| a.iter().any(|r| r["kind"] == "zone"))
        }),
    );
    Ok((records, outside))
}

/// The records the instance's units already render in `zone`.
///
/// # Errors
///
/// As [`crate::infra::select_in`].
pub fn covered(
    root: &Path,
    platform: &Path,
    providers: &Path,
    zone: &str,
) -> Result<Vec<Rendered>, crate::infra::Error> {
    let mut out = Vec::new();
    for unit in crate::infra::select_in(root, platform, providers, &[])? {
        let Some(c) = unit.config.as_ref() else {
            continue;
        };
        let Some(s) = c.site.as_ref().filter(|s| s.workload.is_none()) else {
            continue;
        };
        let items =
            crate::sites::apex::derive(crate::sites::apex::EXPANDER, &c.config, &c.home, s).0;
        out.extend(rendered(&items, zone));
    }
    Ok(out)
}
