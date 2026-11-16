//! The one CHANGELOG.md, rendered whole from first-parent history and release tags; see .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §7.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use regex::Regex;
use serde::Deserialize;

use super::lines::{self, Commit, RELEASE_PREFIX};

/// A unit as a past `releases.yaml` declared it: only what the changelog needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PastUnit {
    /// Path prefixes the unit ships.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Paths the unit does not ship.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The unit's name before a rename.
    #[serde(default)]
    pub previous: Option<String>,
}

#[derive(Deserialize)]
struct Past {
    #[serde(default)]
    units: BTreeMap<String, PastUnit>,
}

/// The units of a past `releases.yaml`, or `None` for empty or unparsable text.
#[must_use]
pub fn past_units(text: &str) -> Option<BTreeMap<String, PastUnit>> {
    if text.trim().is_empty() {
        return None;
    }
    serde_saphyr::from_str::<Past>(text).ok().map(|p| p.units)
}

/// The names of the units whose shipped paths `commit` touched.
#[must_use]
pub fn units_of(commit: &Commit, units: &BTreeMap<String, PastUnit>) -> Vec<String> {
    units
        .iter()
        .filter(|(_, u)| lines::touches_shipped(commit, &u.paths, &u.exclude))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The `(old, new)` pairs where `after` renames a unit `before` had and `after` no longer has.
#[must_use]
pub fn renames(
    before: &BTreeMap<String, PastUnit>,
    after: &BTreeMap<String, PastUnit>,
) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = after
        .iter()
        .filter_map(|(name, u)| {
            let old = u.previous.as_ref()?;
            (before.contains_key(old) && !after.contains_key(old) && !before.contains_key(name))
                .then(|| (old.clone(), name.clone()))
        })
        .collect();
    pairs.sort();
    pairs
}

/// One first-parent commit, the units it released under, and the renames it made.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The commit.
    pub commit: Commit,
    /// Unit names as declared when it merged.
    pub units: Vec<String>,
    /// `(old, new)` unit names it renamed.
    pub renames: Vec<(String, String)>,
}

/// A release tag and the UTC date, `YYYY-MM-DD`, of the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    /// The tag name, `<unit>/v<X.Y.Z>`.
    pub tag: String,
    /// The commit's UTC date.
    pub date: String,
}

/// A commit's line with its units as a ` · ` suffix.
#[must_use]
pub fn entry(repository: &str, merged: &Merged) -> String {
    let line = lines::line(repository, &merged.commit);
    if merged.units.is_empty() {
        line
    } else {
        format!("{line} · {}", merged.units.join(", "))
    }
}

/// The line recording a unit rename.
#[must_use]
pub fn rename_line(renames: &[(String, String)]) -> String {
    let pairs: Vec<String> = renames
        .iter()
        .map(|(old, new)| format!("{old} → {new}"))
        .collect();
    format!("- units renamed · {}", pairs.join(", "))
}

type Version = (u64, u64, u64);

/// `(date, unit, version)` of a release tag, or `None` when it isn't one.
fn parse_tag(re: &Regex, tagged: &Tagged) -> Option<(String, String, Version)> {
    let c = re.captures(&tagged.tag)?;
    let part = |i: usize| c.get(i)?.as_str().parse::<u64>().ok();
    let version = (part(2)?, part(3)?, part(4)?);
    Some((tagged.date.clone(), c[1].to_string(), version))
}

/// The whole `CHANGELOG.md`: days newest first, each with its releases and merged commits.
///
/// # Panics
///
/// Never: its pattern is a constant.
#[must_use]
pub fn render(repository: &str, merged: &[Merged], tags: &[Tagged]) -> String {
    let re = Regex::new(r"^(.+)/v([0-9]+)\.([0-9]+)\.([0-9]+)$").expect("valid regex");
    let mut released: BTreeMap<String, Vec<(String, Version)>> = BTreeMap::new();
    for (date, unit, version) in tags.iter().filter_map(|t| parse_tag(&re, t)) {
        released.entry(date).or_default().push((unit, version));
    }
    for rel in released.values_mut() {
        rel.sort();
    }
    let mut commits: BTreeMap<&str, Vec<&Merged>> = BTreeMap::new();
    for m in merged
        .iter()
        .filter(|m| !m.commit.subject.starts_with(RELEASE_PREFIX))
    {
        commits.entry(m.commit.date.as_str()).or_default().push(m);
    }
    let days: BTreeSet<&str> = released
        .keys()
        .map(String::as_str)
        .chain(commits.keys().copied())
        .collect();

    let mut out = String::from("# Changelog\n");
    for day in days.into_iter().rev() {
        let _ = writeln!(out, "\n## {day}");
        if let Some(rel) = released.get(day) {
            out.push_str("\n### Released\n\n");
            for (unit, (x, y, z)) in rel {
                let _ = writeln!(out, "- {unit} v{x}.{y}.{z}");
            }
        }
        if let Some(list) = commits.get(day) {
            out.push_str("\n### Merged\n\n");
            for m in list {
                out.push_str(&entry(repository, m));
                out.push('\n');
                if !m.renames.is_empty() {
                    out.push_str(&rename_line(&m.renames));
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://example.test/r";

    fn commit(ch: char, date: &str, subject: &str, paths: &[&str]) -> Commit {
        Commit {
            sha: ch.to_string().repeat(40),
            date: date.into(),
            subject: subject.into(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
        }
    }

    fn unit(paths: &[&str], exclude: &[&str], previous: Option<&str>) -> PastUnit {
        PastUnit {
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            exclude: exclude.iter().map(|p| (*p).to_string()).collect(),
            previous: previous.map(String::from),
        }
    }

    fn tag(tag: &str, date: &str) -> Tagged {
        Tagged {
            tag: tag.into(),
            date: date.into(),
        }
    }

    #[test]
    fn past_units_reads_paths_exclude_and_previous_and_ignores_other_keys() {
        let text = "description: x\ntiers:\n  a: b\nunits:\n  a:\n    manifest: Cargo.toml\n    cargo: true\n    paths: [src/a/]\n    exclude: [docs/]\n  b:\n    previous: a\n";
        let units = past_units(text).unwrap();
        assert_eq!(units["a"], unit(&["src/a/"], &["docs/"], None));
        assert_eq!(units["b"], unit(&[], &[], Some("a")));
    }

    #[test]
    fn past_units_is_none_for_empty_or_unparsable_text() {
        assert!(past_units("").is_none());
        assert!(past_units("  \n").is_none());
        assert!(past_units("units: [").is_none());
    }

    #[test]
    fn units_of_names_each_unit_whose_shipped_paths_changed() {
        let units = BTreeMap::from([
            ("b".to_string(), unit(&["src/b/"], &[], None)),
            ("a".to_string(), unit(&["src/a/"], &["docs/"], None)),
            ("c".to_string(), unit(&["src/c/"], &[], None)),
        ]);
        let c = commit('a', "2026-01-01", "x", &["src/b/f.rs", "src/a/docs/x.md"]);
        assert_eq!(units_of(&c, &units), vec!["b".to_string()]);
        let c = commit('a', "2026-01-01", "x", &["src/b/f.rs", "src/a/f.rs"]);
        assert_eq!(units_of(&c, &units), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn renames_are_found_only_where_the_old_unit_goes_away() {
        let before = BTreeMap::from([("a".to_string(), unit(&[], &[], None))]);
        let kept = BTreeMap::from([
            ("a".to_string(), unit(&[], &[], None)),
            ("b".to_string(), unit(&[], &[], Some("a"))),
        ]);
        assert_eq!(renames(&before, &kept), Vec::<(String, String)>::new());
        let gone = BTreeMap::from([("b".to_string(), unit(&[], &[], Some("a")))]);
        assert_eq!(
            renames(&before, &gone),
            vec![("a".to_string(), "b".to_string())]
        );
        let unknown = BTreeMap::from([("b".to_string(), unit(&[], &[], Some("zzz")))]);
        assert_eq!(renames(&before, &unknown), Vec::<(String, String)>::new());
    }

    #[test]
    fn entry_links_a_pr_or_a_commit_and_suffixes_its_units() {
        let pr = Merged {
            commit: commit('a', "2026-01-01", "feat(a): one (#1)", &[]),
            units: vec!["a".into(), "b".into()],
            renames: vec![],
        };
        assert_eq!(
            entry(REPO, &pr),
            "- feat(a): one ([#1](https://example.test/r/pull/1)) · a, b"
        );
        let direct = Merged {
            commit: commit('b', "2026-01-01", "fix: two (!7)", &[]),
            units: vec![],
            renames: vec![],
        };
        assert_eq!(
            entry(REPO, &direct),
            format!(
                "- fix: two (!7) ([bbbbbbb](https://example.test/r/commit/{}))",
                "b".repeat(40)
            )
        );
    }

    fn sample() -> (Vec<Merged>, Vec<Tagged>) {
        let merged = vec![
            Merged {
                commit: commit('a', "2026-01-01", "feat(a): one (#1)", &[]),
                units: vec!["a".into()],
                renames: vec![],
            },
            Merged {
                commit: commit('b', "2026-01-02", "fix: two (!7)", &[]),
                units: vec![],
                renames: vec![("a".into(), "b".into())],
            },
            Merged {
                commit: commit('c', "2026-01-02", "chore(release): a v0.1.1", &[]),
                units: vec![],
                renames: vec![],
            },
        ];
        let tags = vec![
            tag("a/v0.1.1", "2026-01-02"),
            tag("a/v0.1.0", "2026-01-01"),
            tag("junk", "2026-01-02"),
        ];
        (merged, tags)
    }

    #[test]
    fn render_matches_the_example_exactly() {
        let (merged, tags) = sample();
        let expected = format!(
            "# Changelog\n\n## 2026-01-02\n\n### Released\n\n- a v0.1.1\n\n### Merged\n\n- fix: two (!7) ([bbbbbbb](https://example.test/r/commit/{b}))\n- units renamed · a → b\n\n## 2026-01-01\n\n### Released\n\n- a v0.1.0\n\n### Merged\n\n- feat(a): one ([#1](https://example.test/r/pull/1)) · a\n",
            b = "b".repeat(40)
        );
        assert_eq!(render(REPO, &merged, &tags), expected);
    }

    #[test]
    fn render_skips_release_commits_and_unparsable_tags() {
        let (merged, tags) = sample();
        let out = render(REPO, &merged, &tags);
        assert!(!out.contains("chore(release)"));
        assert!(!out.contains("junk"));
        let only_release = render(REPO, &merged[2..], &[tag("nope", "2026-01-05")]);
        assert_eq!(only_release, "# Changelog\n");
    }

    #[test]
    fn render_is_the_same_on_every_call() {
        let (merged, tags) = sample();
        assert_eq!(render(REPO, &merged, &tags), render(REPO, &merged, &tags));
    }
}
