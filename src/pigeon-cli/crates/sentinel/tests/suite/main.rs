//! pigeon-sentinel's integration tests, one module per mechanism; see
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md`.

mod change;
mod cli;
mod definitions;
mod eviction;
mod policies;
mod tree;
