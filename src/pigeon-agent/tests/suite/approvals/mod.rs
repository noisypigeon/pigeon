//! Approval tiers, pages, asks, grants and their ledger, mandates and platforms. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

#[allow(clippy::module_inception)]
mod approvals;
mod approve;
mod asks_approval;
mod change_asks;
mod grants;
mod ledger;
mod mandates;
mod platform_job;
mod platform_shape;
mod platforms;
