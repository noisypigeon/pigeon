//! Remote infrastructure: git on Artifacts, R2 state, site sync and the Cloudflare token. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod git;
mod gitcreds;
mod infra_state;
mod sites;
mod workers_cloudflare;
