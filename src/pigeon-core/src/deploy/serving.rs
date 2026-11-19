//! Whether the platform serves after a deploy of `main`, and the deploy pipeline's paths; see
//! .claude/decisions/2026-11-19-security-auto-held-when-platform-down.md §1 and §2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The code that runs a deploy: a failed deploy whose range touches one holds Auto.
pub const PIPELINE: &[&str] = &[
    "src/pigeon-core/src/deploy/**",
    "src/pigeon-core/src/control.rs",
    "src/pigeon-agent/src/ship.rs",
    "src/pigeon-agent/src/jobs.rs",
    "src/pigeon-agent/src/workers/**",
    "src/pigeon-agent/src/worker/main_run.rs",
    "Dockerfile",
    "pigeon-start",
    ".dockerignore",
];

/// Whether any of `paths` is in [`PIPELINE`].
#[must_use]
pub fn touches_pipeline<S: AsRef<str>>(paths: &[S]) -> bool {
    paths.iter().any(|p| {
        PIPELINE
            .iter()
            .any(|g| crate::approval::tiers::matches(g, p.as_ref()))
    })
}

/// What the deploy job found serving once its deploy ended.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Serving {
    /// The lease's colour, or `pigeon-control` without a lease row.
    #[serde(default)]
    pub lease: String,
    /// Whether the platform host's `/healthz` answered `ok`, through the front.
    #[serde(default)]
    pub front: bool,
    /// Each colour Worker, and whether its own host's `/healthz` answered `ok`.
    #[serde(default)]
    pub colors: BTreeMap<String, bool>,
}

impl Serving {
    /// The colours that are up.
    #[must_use]
    pub fn up(&self) -> Vec<&str> {
        self.colors
            .iter()
            .filter(|(_, ok)| **ok)
            .map(|(c, _)| c.as_str())
            .collect()
    }

    /// Why the platform can't serve, if it can't: no colour up, or the front not forwarding.
    #[must_use]
    pub fn down(&self) -> Option<String> {
        if self.up().is_empty() {
            Some("no colour answers its /healthz".into())
        } else if !self.front {
            Some(format!(
                "the platform host doesn't reach a healthy colour (the lease names {})",
                self.lease
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serving(front: bool, colors: &[(&str, bool)]) -> Serving {
        Serving {
            lease: "pigeon-control".into(),
            front,
            colors: colors.iter().map(|(c, b)| ((*c).into(), *b)).collect(),
        }
    }

    #[test]
    fn the_platform_is_down_only_with_no_colour_up_or_no_front() {
        assert_eq!(serving(true, &[("pigeon-control", true)]).down(), None);
        assert_eq!(
            serving(true, &[("pigeon-control", true), ("pigeon-green", false)]).down(),
            None
        );
        assert!(serving(true, &[("pigeon-control", false)]).down().is_some());
        assert!(serving(true, &[]).down().is_some());
        assert!(serving(false, &[("pigeon-control", true)]).down().is_some());
    }

    #[test]
    fn the_pipeline_is_the_deploy_code_and_the_job_image() {
        assert!(touches_pipeline(&["src/pigeon-agent/src/workers/ship.rs"]));
        assert!(touches_pipeline(&["README.md", "pigeon-start"]));
        assert!(!touches_pipeline(&["src/pigeon-agent/src/worker/push.rs"]));
        assert!(!touches_pipeline::<&str>(&[]));
    }
}
