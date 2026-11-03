//! Moving a unit's state to a new path; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md` (2026-11-03 amendment).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::unit::Unit;

/// The moves file, from the repository root.
pub const FILE: &str = ".config/moves.yaml";

/// The state file inside a unit's state directory.
pub const STATE: &str = "terraform.tfstate";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Moves {
    #[expect(dead_code, reason = "a description is required, never read")]
    description: String,
    #[serde(default)]
    moves: Vec<Move>,
}

/// One unit's move.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Move {
    /// The old unit path.
    pub from: String,
    /// The new unit path.
    pub to: String,
}

/// The moves in `<root>/.config/moves.yaml`; none when it is absent.
///
/// # Errors
///
/// A message if the file exists but does not parse.
pub fn load(root: &Path) -> Result<Vec<Move>, String> {
    let path = root.join(FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_saphyr::from_str::<Moves>(&text)
        .map(|m| m.moves)
        .map_err(|e| e.to_string())
}

/// The move into `unit`, if any.
#[must_use]
pub fn into<'a>(moves: &'a [Move], unit: &str) -> Option<&'a Move> {
    moves.iter().find(|m| m.to == unit)
}

/// Problems with the moves, as `(path, message)`.
#[must_use]
pub fn validate(root: &Path, units: &[Unit]) -> Vec<(String, String)> {
    let moves = match load(root) {
        Ok(m) => m,
        Err(e) => return vec![(FILE.into(), e)],
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let area = |p: &str| p.split('/').next().unwrap_or_default().to_string();
    for m in &moves {
        let at = format!("{FILE}: {} -> {}", m.from, m.to);
        if area(&m.from) != area(&m.to) || !crate::unit::AREAS.contains(&area(&m.to).as_str()) {
            out.push((
                at.clone(),
                "a move stays within infra/ or within workloads/".into(),
            ));
        }
        if !units.iter().any(|u| u.path == m.to) {
            out.push((at.clone(), "`to` must be a unit".into()));
        }
        if units.iter().any(|u| u.path == m.from) {
            out.push((at.clone(), "`from` must no longer be a unit".into()));
        }
        for p in [&m.from, &m.to] {
            if !seen.insert(p.clone()) {
                out.push((at.clone(), format!("`{p}` is in two moves")));
            }
        }
    }
    out
}
