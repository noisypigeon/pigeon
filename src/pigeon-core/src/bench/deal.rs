//! Fan-out lanes for a session's checks.

use super::MAX_PER_SESSION;
use serde::{Deserialize, Serialize};

/// A named check and the argv that runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// The check's name.
    pub name: String,
    /// The command it runs.
    pub argv: Vec<String>,
}

/// The lane a check runs in: 2 wasm, 1 tests, 0 the rest.
#[must_use]
pub fn lane(check: &Check) -> u8 {
    if check.name.contains("wasm") {
        2
    } else {
        u8::from(check.name.starts_with("test"))
    }
}

/// Deals checks across `benches` lists by lane, keeping input order.
#[must_use]
pub fn deal(checks: &[Check], benches: u8) -> Vec<Vec<Check>> {
    let n = benches.clamp(1, MAX_PER_SESSION);
    let mut lists = vec![Vec::new(); usize::from(n)];
    for c in checks {
        lists[usize::from(lane(c).min(n - 1))].push(c.clone());
    }
    lists
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str) -> Check {
        Check {
            name: name.to_string(),
            argv: vec![],
        }
    }

    fn names(l: &[Check]) -> Vec<&str> {
        l.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn lanes() {
        assert_eq!(lane(&c("fmt")), 0);
        assert_eq!(lane(&c("clippy pigeon")), 0);
        assert_eq!(lane(&c("test pigeon-backplane: lib")), 1);
        assert_eq!(lane(&c("wasm clippy")), 2);
        assert_eq!(lane(&c("policy.check")), 0);
    }

    fn all() -> Vec<Check> {
        ["fmt", "test a", "wasm clippy", "clippy pigeon", "test b"]
            .iter()
            .map(|n| c(n))
            .collect()
    }

    #[test]
    fn deal_three() {
        let d = deal(&all(), 3);
        assert_eq!(d.len(), 3);
        assert_eq!(names(&d[0]), ["fmt", "clippy pigeon"]);
        assert_eq!(names(&d[1]), ["test a", "test b"]);
        assert_eq!(names(&d[2]), ["wasm clippy"]);
    }

    #[test]
    fn deal_two() {
        let d = deal(&all(), 2);
        assert_eq!(d.len(), 2);
        assert_eq!(names(&d[0]), ["fmt", "clippy pigeon"]);
        assert_eq!(names(&d[1]), ["test a", "wasm clippy", "test b"]);
    }

    #[test]
    fn deal_one() {
        let d = deal(&all(), 1);
        assert_eq!(d.len(), 1);
        assert_eq!(
            names(&d[0]),
            ["fmt", "test a", "wasm clippy", "clippy pigeon", "test b"]
        );
    }

    #[test]
    fn deal_zero_is_one() {
        assert_eq!(deal(&all(), 0).len(), 1);
    }

    #[test]
    fn deal_five_is_three() {
        let d = deal(&all(), 5);
        assert_eq!(d.len(), 3);
        assert_eq!(names(&d[2]), ["wasm clippy"]);
    }

    #[test]
    fn deal_empty_lists() {
        let d = deal(&[], 3);
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(Vec::is_empty));
    }
}
