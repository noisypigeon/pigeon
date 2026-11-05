//! The engines backplane composes.

use serde::Serialize;

use crate::wire::Engine;

/// An engine's name, version and role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineInfo {
    /// The engine.
    pub engine: Engine,
    /// Its crate version.
    pub version: &'static str,
    /// `policy`, `action` or `planned`.
    pub role: &'static str,
}

/// Every engine backplane composes.
#[must_use]
pub fn engines() -> Vec<EngineInfo> {
    vec![
        EngineInfo {
            engine: Engine::Sentinel,
            version: sentinel::VERSION,
            role: "policy",
        },
        EngineInfo {
            engine: Engine::Cobot,
            version: cobot::VERSION,
            role: "action",
        },
        EngineInfo {
            engine: Engine::Gantry,
            version: gantry::VERSION,
            role: "planned",
        },
    ]
}
