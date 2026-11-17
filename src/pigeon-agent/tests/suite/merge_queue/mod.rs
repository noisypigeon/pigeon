//! The merge queue: one merge at a time, turns, yields and gates. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod merge_front;
mod merge_gate_early;
mod merge_holder;
mod merge_kept;
mod merge_release_turn;
mod merge_released;
mod merges;
