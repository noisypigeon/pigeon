//! The engines the agent composes.

use serde::Serialize;

use crate::wire::Engine;

/// An engine's name, version and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineInfo {
    /// The engine.
    pub engine: Engine,
    /// Its version: its crate's, or for the judge its binary's, `not installed` when absent.
    pub version: String,
    /// `policy`, `action` or `planned`.
    pub role: &'static str,
}

/// Every engine the agent composes; the judge's version is its binary's, found as
/// [`crate::eviction::locate`] finds it.
#[must_use]
pub fn engines() -> Vec<EngineInfo> {
    let judge = crate::eviction::locate(&|k| std::env::var(k).ok());
    vec![
        EngineInfo {
            engine: Engine::Eviction,
            version: crate::eviction::version(&judge).unwrap_or_else(|| "not installed".into()),
            role: "policy",
        },
        EngineInfo {
            engine: Engine::Infra,
            version: crate::VERSION.into(),
            role: "action",
        },
    ]
}
