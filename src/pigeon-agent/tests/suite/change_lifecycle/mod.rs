//! A change's life: its states, restacks, eviction, reconciles, overlaps and the git door. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod changes;
mod eviction;
mod gitdoor;
mod overlaps;
mod reconcile;
