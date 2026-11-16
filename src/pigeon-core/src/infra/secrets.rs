//! `secrets.config.yaml`'s name and the shape of its encrypted values, as core checks them; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §4.

/// The secrets file name inside a unit.
pub const FILE: &str = "secrets.config.yaml";

/// Whether `value` is an encrypted value.
#[must_use]
pub fn is_encrypted(value: &str) -> bool {
    crate::names::enc_body(value).is_some() && value.ends_with(']')
}
