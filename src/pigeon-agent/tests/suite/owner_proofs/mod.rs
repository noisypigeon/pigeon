//! The owner's proofs: passkeys, TOTP, enrolment, Web Push and devices. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod devices;
mod enroll;
mod passkeys;
mod push;
mod totp;
