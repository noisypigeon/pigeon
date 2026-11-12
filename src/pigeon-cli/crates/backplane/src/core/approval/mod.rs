//! The `approval` domain: tiers, grants, mandates, passkeys and TOTP, as judgements and
//! transitions; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod grants;
pub mod mandates;
pub mod passkeys;
pub mod tiers;
pub mod totp;

#[cfg(test)]
mod tests;
