//! The `change` domain: a change's lifecycle, its jobs, restack and merge gate, and the merge
//! queue, as transitions; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod asks;
pub mod breaker;
pub mod eviction;
pub mod forecast;
mod gate;
mod jobs;
mod lifecycle;
pub mod queue;
mod restack;
pub mod stage;
mod types;

pub use jobs::{
    LOST_BACKOFF, MAX_START_FAILURES, STALL_SECS, stalled, start_backoff, undispatched,
};
pub use lifecycle::{Step, step};
pub use types::*;

#[cfg(test)]
mod tests;
