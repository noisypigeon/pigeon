//! The `sites` domain: a site's layout and its assembled project, from what the shell reads; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod assemble;
pub mod layout;

#[cfg(test)]
mod tests;
