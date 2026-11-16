//! pigeon's integration tests, one module per mechanism; see
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md`.

mod agent;
mod aliases;
mod cli;
mod dev;
mod eviction;
mod harness;
mod infra;
mod session_sh;
mod signing;
