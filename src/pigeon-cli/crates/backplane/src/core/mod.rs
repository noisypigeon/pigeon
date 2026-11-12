//! Backplane's functional core: types, transitions and judgements, with no I/O, clock or
//! `&mut` receiver; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §3.

pub mod approval;
pub mod change;
