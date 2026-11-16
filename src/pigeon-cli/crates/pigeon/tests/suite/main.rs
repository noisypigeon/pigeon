//! pigeon's integration tests, one module per mechanism; see
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md`.

mod backplane;
mod cli;
mod cobot;
mod harness;
mod sentinel;
mod session_sh;
mod signing;
