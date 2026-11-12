//! Moving a unit's state to a new path, read from disk; the checks are
//! [`crate::core::infra::moves`]; see
//! `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md` (2026-11-03 amendment).

use std::fs;
use std::path::Path;

use crate::cobot::unit::Unit;

pub use crate::core::infra::moves::*;

/// The moves in `<root>/.config/moves.yaml`, then the instance manifest's; none when both
/// are absent.
///
/// # Errors
///
/// A message if the file exists but does not parse.
pub fn load(root: &Path) -> Result<Vec<Move>, String> {
    let mut moves = file_moves(root)?;
    moves.extend(crate::cobot::manifest::moves(root));
    Ok(moves)
}

fn file_moves(root: &Path) -> Result<Vec<Move>, String> {
    let path = root.join(FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    parse(&text)
}

/// Problems with the moves, as `(path, message)`.
#[must_use]
pub fn validate(root: &Path, units: &[Unit]) -> Vec<(String, String)> {
    problems(
        file_moves(root),
        &crate::cobot::manifest::moves(root),
        units,
    )
}
