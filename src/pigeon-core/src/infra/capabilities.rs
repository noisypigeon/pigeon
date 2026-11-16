//! A provider catalog's declared capabilities, as core checks units and the ledger against
//! them; see `.claude/decisions/2026-11-04-development-platform-instance-separation.md` §2–§3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One capability: provider permissions, minted at one scope.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    /// The provider's permission names.
    pub permissions: Vec<String>,
    /// What it is minted on, such as `zone` or `bucket`.
    pub scope: String,
}

/// A provider's declared capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    /// The provider, the catalog directory's name.
    pub provider: String,
    /// The capabilities file, from the root.
    pub path: String,
    /// The capabilities, by name.
    pub capabilities: BTreeMap<String, Capability>,
}
