//! The `change` domain: a change's lifecycle, its jobs, restack and merge gate, and the merge
//! queue, as transitions; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod eviction;
mod gate;
mod jobs;
mod lifecycle;
pub mod queue;
mod restack;
pub mod stage;
mod types;

pub use lifecycle::{Step, step};
pub use types::*;

#[cfg(test)]
mod tests;
