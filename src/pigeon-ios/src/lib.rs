//! The iOS app's checks that run on Linux: its manifest and the operations its Swift names. It
//! never builds Swift; see .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §1.

use std::io;
use std::path::{Path, PathBuf};

/// The Swift targets' source directories, as `project.yml` names them.
pub const SWIFT_DIRS: [&str; 3] = ["PigeonCore", "PigeonCoreTests", "Pigeon"];

/// The app's directory, `src/pigeon-ios`.
#[must_use]
pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.swift` file under `dir`, sorted.
///
/// # Errors
///
/// When a directory can't be read.
pub fn swift_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d)? {
            let path = entry?.path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "swift") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Each quoted `"<domain>.<name>"` in `source`, with one dot and a domain in `domains`, in order.
#[must_use]
pub fn operation_literals<'a>(source: &'a str, domains: &[&str]) -> Vec<&'a str> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    for (i, _) in source.match_indices('"') {
        let start = i + 1;
        let len = bytes[start..]
            .iter()
            .take_while(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || **b == b'_' || **b == b'.'
            })
            .count();
        if bytes.get(start + len) != Some(&b'"') {
            continue;
        }
        let text = &source[start..start + len];
        if let Some((domain, name)) = text.split_once('.')
            && !name.is_empty()
            && !name.contains('.')
            && domains.contains(&domain)
        {
            out.push(text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::operation_literals;

    #[test]
    fn only_quoted_names_in_a_known_domain_count() {
        let swift = r#"case "change.merge": x; Image(systemName: "chevron.down"); "change."; "change.pigeon.dev" "#;
        assert_eq!(operation_literals(swift, &["change"]), ["change.merge"]);
    }

    #[test]
    fn interpolation_and_json_fixtures_are_read() {
        let swift = r#"x("\(a ? "b" : "c")"); """{"operation":"change.approve"}""""#;
        assert_eq!(operation_literals(swift, &["change"]), ["change.approve"]);
    }
}
