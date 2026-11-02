//! Violations and the summary of a run.

use std::fmt;

/// One broken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The file (relative to the root) or target the violation is in.
    pub path: String,
    /// The 1-based line, where one applies.
    pub line: Option<usize>,
    /// The policy's name.
    pub policy: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}: {}", self.path, self.policy, self.message),
            None => write!(f, "{}: {}: {}", self.path, self.policy, self.message),
        }
    }
}

/// The outcome of checking every policy.
#[derive(Debug, Default)]
pub struct Report {
    /// Policies that ran.
    pub ran: usize,
    /// Pull-request policies skipped because no pull request was given.
    pub skipped: usize,
    /// Every violation, in the order found.
    pub violations: Vec<Violation>,
}

impl Report {
    /// Whether every policy held.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// A one-line summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let skipped = if self.skipped > 0 {
            format!(", {} skipped", self.skipped)
        } else {
            String::new()
        };
        if self.passed() {
            format!("sentinel: {} policies hold{skipped}", self.ran)
        } else {
            format!(
                "sentinel: {} violation(s) across {} policies{skipped}",
                self.violations.len(),
                self.ran
            )
        }
    }
}
