//! ADRs and the paths they govern: each ADR's `* Governs:` line lists the globs it rules; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §4.

use serde::Serialize;

/// Where ADRs live.
pub const DIR: &str = ".claude/decisions";

/// One ADR: its file, its title and the globs it governs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decision {
    /// Its file name.
    pub file: String,
    /// Its title, the first line's heading.
    pub title: String,
    /// Its `Governs` globs; none without the line.
    pub governs: Vec<String>,
}

/// The ADR `file` whose text is `text`.
#[must_use]
pub fn parse(file: &str, text: &str) -> Decision {
    let title = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("## "))
        .unwrap_or_default()
        .trim()
        .to_string();
    let governs = text
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .find_map(|l| l.strip_prefix("* Governs:"))
        .map(|v| {
            v.split(',')
                .map(|g| g.trim().trim_matches('`').trim().to_string())
                .filter(|g| !g.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Decision {
        file: file.to_string(),
        title,
        governs,
    }
}

/// The ADRs among `all` that govern `path`.
#[must_use]
pub fn governing<'a>(all: &'a [Decision], path: &str) -> Vec<&'a Decision> {
    let path = path.trim_start_matches("./");
    all.iter()
        .filter(|d| d.governs.iter().any(|g| crate::approvals::matches(g, path)))
        .collect()
}
