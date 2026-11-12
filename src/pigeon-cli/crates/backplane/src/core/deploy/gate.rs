//! How much of the full set a head's merge gate runs; see the 2026-11-12 amendment on the merge
//! gate to `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md`.

use serde::{Deserialize, Serialize};

/// A head's merge gate: scoped, skipping the full test run, or full, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "why", rename_all = "snake_case")]
pub enum Gate {
    /// The test map chose the head's tests: the gate skips `cargo test --workspace`.
    Scoped,
    /// The gate runs the full test suite, for the reason given.
    Full(String),
}

impl Default for Gate {
    /// Full, as every gate was before the amendment.
    fn default() -> Self {
        Self::Full("no gate was reported".into())
    }
}

/// The most packages a scoped gate's tests may span.
pub const MAX_PACKAGES: usize = 2;
