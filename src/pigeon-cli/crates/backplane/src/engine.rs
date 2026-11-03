//! The engines backplane composes.

use serde::Serialize;

/// Who runs an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Backplane itself.
    Backplane,
    /// Policy and its enforcement.
    Sentinel,
    /// Actions: infrastructure, releases.
    Cobot,
    /// Planned: scheduling and data operations.
    Gantry,
}

impl Engine {
    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Backplane => "backplane",
            Self::Sentinel => "sentinel",
            Self::Cobot => "cobot",
            Self::Gantry => "gantry",
        }
    }
}

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
