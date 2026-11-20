//! Workers: specs, deploys and ship outputs. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod deployable;
#[allow(clippy::module_inception)]
mod workers;
mod workers_ship;
