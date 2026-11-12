//! The `app` domain: the iOS app's devices and requests; see
//! `.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md`.

pub mod approvals;
pub mod devices;
pub mod push;

#[cfg(test)]
mod tests;
