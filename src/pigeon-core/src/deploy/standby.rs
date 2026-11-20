//! The colour a Worker deploy goes to: the one the lease doesn't name; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its step 10 amendment.

use serde::{Deserialize, Serialize};

use crate::lease::Lease;

/// What a deploy found on the standby it shipped: checked on its own host, in order, stopping at the first failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// The standby colour.
    pub color: String,
    /// Its own host.
    pub host: String,
    /// The version it shipped.
    pub version: String,
    /// Whether every check passed.
    pub passed: bool,
    /// The first check that failed, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

/// The colour among `colors` a deploy goes to: the first, in sorted order, that `lease` doesn't name. None without a lease, with fewer than two colours, or when the lease names none of them, so today's deploy is unchanged.
#[must_use]
pub fn target<'a>(lease: Option<&Lease>, colors: &'a [String]) -> Option<&'a str> {
    let lease = lease?;
    if colors.len() < 2 || !colors.contains(&lease.color) {
        return None;
    }
    let mut sorted: Vec<&String> = colors.iter().collect();
    sorted.sort();
    sorted
        .into_iter()
        .find(|c| **c != lease.color)
        .map(String::as_str)
}

/// Instance keys that change on every read of the event log; parity leaves them out.
pub const VOLATILE: &[&str] = &["event-cursors", "push-latest", "digest-cursor"];

/// The colours a deploy picks its standby among: each colour unit's Worker, and the platform's Worker `platform` while the lease names it or fewer than two colour units exist.
#[must_use]
pub fn colours(platform: &str, units: &[String], lease: &str) -> Vec<String> {
    let mut v = units.to_vec();
    if (v.len() < 2 || lease == platform) && !v.iter().any(|u| u == platform) {
        v.push(platform.to_string());
    }
    v
}

/// Where a standby's `/healthz/parity` answer differs from the primary's: per object of the primary's, each key whose `replayed` digest on the standby isn't its `held` digest on the primary, and each key the standby couldn't open, as `<object>: <key>`; [`VOLATILE`] keys are left out; and `keys` or `vars differ` when both carry one that differs. Sorted; empty when they agree.
#[must_use]
pub fn parity(standby: &serde_json::Value, primary: &serde_json::Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let mut out = Vec::new();
    let Some(objects) = primary["objects"].as_object() else {
        return out;
    };
    for (object, p) in objects {
        let held = p["held"].as_object().unwrap_or(&empty);
        let theirs = &standby["objects"][object];
        let replayed = theirs["replayed"].as_object().unwrap_or(&empty);
        let keys: std::collections::BTreeSet<&String> =
            held.keys().chain(replayed.keys()).collect();
        for key in keys {
            if VOLATILE.contains(&key.as_str()) {
                continue;
            }
            if key == crate::journal::ERROR {
                let why = held.get(key).and_then(|v| v.as_str()).unwrap_or_default();
                out.push(format!("{object}: journal failing: {why}"));
            } else if held.get(key) != replayed.get(key) {
                out.push(format!("{object}: {key}"));
            }
        }
        for k in theirs["unopened"].as_array().into_iter().flatten() {
            if let Some(k) = k.as_str() {
                out.push(format!("{object}: {k} unopened"));
            }
        }
        // An object a side couldn't read matches nothing, however empty both look.
        for (side, answer) in [("primary", p), ("standby", theirs)] {
            if let Some(e) = answer["error"].as_str() {
                out.push(format!("{object}: the {side} couldn't read it: {e}"));
            }
        }
    }
    // What every colour must hold alike, when both answers carry it. Passkeys are compared by the
    // replayed `passkeys` key here, and live only once adopted, by a promote; see the blue-green
    // ADR's 2026-11-20 deploy-time passkeys amendment.
    for field in ["keys", "vars"] {
        let (theirs, ours) = (&standby[field], &primary[field]);
        if !theirs.is_null() && !ours.is_null() && theirs != ours {
            out.push(format!("{field} differ"));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(c: &str) -> Lease {
        Lease {
            color: c.into(),
            epoch: 1,
            state: "serving".into(),
        }
    }

    #[test]
    fn the_standby_is_the_colour_the_lease_doesnt_name() {
        let two = vec!["pigeon-green".to_string(), "pigeon-blue".to_string()];
        assert_eq!(
            target(Some(&lease("pigeon-green")), &two),
            Some("pigeon-blue")
        );
        assert_eq!(
            target(Some(&lease("pigeon-blue")), &two),
            Some("pigeon-green")
        );
        assert_eq!(target(None, &two), None);
        assert_eq!(target(Some(&lease("pigeon-control")), &two), None);
        assert_eq!(target(Some(&lease("pigeon-green")), &two[..1]), None);
    }

    #[test]
    fn the_platform_worker_is_a_colour_until_two_units_exist() {
        let p = "pigeon-control";
        let v = |s: &[&str]| s.iter().map(|x| (*x).to_string()).collect::<Vec<String>>();
        let green = v(&["pigeon-green"]);
        let both = v(&["pigeon-green", "pigeon-blue"]);
        assert_eq!(colours(p, &green, p), v(&["pigeon-green", p]));
        assert_eq!(colours(p, &green, "pigeon-green"), v(&["pigeon-green", p]));
        assert_eq!(colours(p, &both, "pigeon-green"), both);
        assert_eq!(colours(p, &both, p), v(&["pigeon-green", "pigeon-blue", p]));
    }

    #[test]
    fn parity_names_each_object_key_that_differs_but_never_a_volatile_one() {
        use serde_json::json;
        let primary =
            json!({"objects": {"i": {"held": {"a": "1", "b": "2", "event-cursors": "x"}}}});
        let standby = json!({"objects": {"i": {"replayed": {"a": "1", "b": "3", "c": "4", "event-cursors": "y"}, "unopened": ["d"]}}});
        assert_eq!(
            parity(&standby, &primary),
            vec!["i: b", "i: c", "i: d unopened"]
        );
        let same = json!({"objects": {"i": {"replayed": {"a": "1", "b": "2"}}}});
        assert_eq!(parity(&same, &primary), Vec::<String>::new());
        let with = |v: &str| json!({"objects": {}, "vars": {"PROOFS": v}});
        assert_eq!(parity(&with("x"), &with("passkey")), vec!["vars differ"]);
        assert_eq!(
            parity(&with("passkey"), &json!({"objects": {}})),
            Vec::<String>::new()
        );
        let ring = |v: &str| json!({"objects": {}, "passkeys": [v]});
        assert_eq!(parity(&ring("a"), &ring("b")), Vec::<String>::new());
        let failing = json!({"objects": {"i": {"held": {"journal.error": "too big"}}}});
        assert_eq!(
            parity(&json!({"objects": {}}), &failing),
            vec!["i: journal failing: too big"]
        );
        assert_eq!(crate::journal::error_text("a\n  b"), "a b");
        let unread = json!({"objects": {"i": {"held": {}, "error": "bad key"}}});
        assert_eq!(
            parity(&json!({"objects": {}}), &unread),
            vec!["i: the primary couldn't read it: bad key"]
        );
    }
}
