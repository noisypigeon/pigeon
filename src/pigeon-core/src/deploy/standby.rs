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
}
