//! The approval family of presenters; see .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md.

/// The operation-name prefixes of this family; empty for the rest.
#[allow(dead_code)]
pub const PREFIXES: &[&str] = &[
    "approval.",
    "grant.",
    "mandate.",
    "platform.",
    "pool.",
    "deploy.",
];

/// How many of this family's operations have no presenter yet; each brief that presents some lowers it.
#[allow(dead_code)]
pub const PENDING: usize = 16;

/// The operations this family presents.
pub const NAMES: &[&str] = &[];

/// The view of operation `op`'s result, when this family presents it.
#[must_use]
pub fn view(op: &str, r: &crate::wire::Response) -> Option<serde_json::Value> {
    let _ = (op, r);
    None
}
