//! The engines backplane composes.

use serde::Serialize;

use crate::wire::Engine;

/// An engine's name, version and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineInfo {
    /// The engine.
    pub engine: Engine,
    /// Its version: its crate's, or for sentinel the binary's, `not installed` when absent.
    pub version: String,
    /// `policy`, `action` or `planned`.
    pub role: &'static str,
}

/// Every engine backplane composes; sentinel's version is its binary's, found as
/// [`crate::eviction::locate`] finds it.
#[must_use]
pub fn engines() -> Vec<EngineInfo> {
    let sentinel = crate::eviction::locate(&|k| std::env::var(k).ok());
    vec![
        EngineInfo {
            engine: Engine::Eviction,
            version: crate::eviction::version(&sentinel).unwrap_or_else(|| "not installed".into()),
            role: "policy",
        },
        EngineInfo {
            engine: Engine::Infra,
            version: crate::VERSION.into(),
            role: "action",
        },
    ]
}
