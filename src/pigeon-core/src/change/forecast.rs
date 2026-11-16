//! Conflict forecasts: which of two open changes that would conflict goes first; see
//! `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §5.

/// How far along a change in `state` is: the further along, the sooner it merges.
#[must_use]
pub fn rank(state: &str) -> u8 {
    match state {
        "approved" => 3,
        "checked" => 2,
        "bounded" | "pushed" => 1,
        _ => 0,
    }
}

/// Whether change `mine` (number, state) yields to `theirs`: theirs is further along, or as far
/// and older.
#[must_use]
pub fn yields(mine: (u32, &str), theirs: (u32, &str)) -> bool {
    (rank(theirs.1), std::cmp::Reverse(theirs.0)) > (rank(mine.1), std::cmp::Reverse(mine.0))
}
