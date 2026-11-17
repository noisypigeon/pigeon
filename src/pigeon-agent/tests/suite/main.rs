//! pigeon-agent's integration tests, one module per mechanism; see
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md`.

#[macro_use]
mod harness;
mod approvals;
mod briefs;
mod change_lifecycle;
mod checks;
mod control_plane;
mod infra;
mod infra_remote;
mod instance;
mod merge_queue;
mod owner_proofs;
mod release;
mod sessions;
mod stored_state;
mod workers;
