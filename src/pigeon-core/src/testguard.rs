//! Counting what a change's diff adds to tests, for the tests guard; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §C.2.

use regex::Regex;
use std::sync::LazyLock;

/// What a Rust diff adds to tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Added `#[test]` and `#[tokio::test]` attributes.
    pub added_tests: usize,
    /// Added lines in test paths that pin a per-feature count or list catalog names.
    pub enumerations: usize,
}

static TEST_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*#\[(tokio::)?test\b").expect("valid regex"));
static LEN_FIRST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"assert_eq!\(\s*[^,]*\.(len|count)\(\)\s*,\s*[0-9]+\s*[,)]").expect("valid regex")
});
static NUM_FIRST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"assert_eq!\(\s*[0-9]+\s*,\s*[^,]*\.(len|count)\(\)").expect("valid regex")
});
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[a-z][a-z0-9_]*\.[a-z][a-z0-9_.]*""#).expect("valid regex"));

fn is_test_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    path.contains("/tests/") || file.ends_with("tests.rs")
}

fn is_enumeration(line: &str) -> bool {
    LEN_FIRST.is_match(line) || NUM_FIRST.is_match(line) || NAME.find_iter(line).count() >= 3
}

/// Counts the tests and enumerations added by `diff`, the text of
/// `git diff --unified=0 <range> -- '*.rs'`.
#[must_use]
pub fn count(diff: &str) -> Counts {
    let mut counts = Counts::default();
    let mut test_path = false;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            test_path = path.strip_prefix("b/").is_some_and(is_test_path);
        } else if let Some(added) = line.strip_prefix('+') {
            if TEST_ATTR.is_match(added) {
                counts.added_tests += 1;
            }
            if test_path && is_enumeration(added) {
                counts.enumerations += 1;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_added_test_attributes() {
        let diff = "+++ b/src/x/src/lib.rs\n+#[test]\n+fn a() {}\n+    #[test]\n+#[tokio::test]\n-#[test]\n";
        assert_eq!(count(diff).added_tests, 3);
    }

    #[test]
    fn counts_len_asserts_only_in_test_paths() {
        let line = "+    assert_eq!(ops.len(), 49);\n";
        let test = format!("+++ b/src/x/tests/suite/a.rs\n{line}");
        let other = format!("+++ b/src/x/src/lib.rs\n{line}");
        assert_eq!(count(&test).enumerations, 1);
        assert_eq!(count(&other).enumerations, 0);
    }

    #[test]
    fn counts_number_first_assert_in_a_tests_file() {
        let diff = "+++ b/src/x/src/foo_tests.rs\n+    assert_eq!(7, n.count());\n";
        assert_eq!(count(diff).enumerations, 1);
    }

    #[test]
    fn counts_three_catalog_names() {
        let three =
            "+++ b/src/x/tests/a.rs\n+[\"change.status\", \"change.list\", \"deploy.status\"]\n";
        let two = "+++ b/src/x/tests/a.rs\n+[\"change.status\", \"change.list\"]\n";
        assert_eq!(count(three).enumerations, 1);
        assert_eq!(count(two).enumerations, 0);
    }
}
