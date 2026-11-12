//! The `sites` domain: a site's layout, its assembled project, what it publishes and its live
//! checks, from what the shell reads; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod assemble;
pub mod checks;
pub mod layout;
pub mod publish;

#[cfg(test)]
mod tests;
